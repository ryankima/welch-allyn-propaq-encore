use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::{egui, App};
use egui_plot::{Line, Plot, PlotPoints};
use serialport;

const WINDOW_SIZE: usize = 1000;
const NUM_CHANNELS: usize = 3;
const SPIKE_THRESHOLD_MULTIPLIER: f64 = 3.0;
const MEDIAN_WINDOW_SIZE: usize = 5;

// Heart rate detection constants
const HR_MIN_BPM: f64 = 40.0;
const HR_MAX_BPM: f64 = 200.0;
const HR_DETECTION_WINDOW: f64 = 10.0; // seconds
const PEAK_MIN_DISTANCE: f64 = 0.3; // minimum seconds between peaks (200 BPM)
const ECG_SAMPLE_RATE_HZ: f64 = 250.0; // Typical for ProPaq Encore
const PPG_SAMPLE_RATE_HZ: f64 = 125.0;
const BP_SAMPLE_RATE_HZ: f64 = 125.0;

#[derive(Clone)]
struct TimestampedValue {
    timestamp: f64, // Time in seconds
    value: f64,
}

#[derive(Clone)]
struct HeartRateDetector {
    peaks: VecDeque<f64>, // timestamps of detected peaks
    last_hr: f64,
    signal_quality: f64,
}

impl HeartRateDetector {
    fn new() -> Self {
        Self {
            peaks: VecDeque::new(),
            last_hr: 0.0,
            signal_quality: 0.0,
        }
    }

    fn detect_peaks(&mut self, data: &VecDeque<TimestampedValue>, channel: usize) -> f64 {
        if data.len() < 20 {
            return self.last_hr;
        }

        // Different algorithms for different channels
        let new_peaks = if channel == 0 {
            self.detect_ppg_peaks(data) // Channel 0 - PPG/Plethysmograph
        } else {
            self.detect_ecg_peaks(data) // Channel 1 - ECG
        };

        // Add new peaks to our collection
        for peak_time in new_peaks {
            self.peaks.push_back(peak_time);
        }

        // Remove old peaks (older than detection window)
        let current_time = data.back().map(|v| v.timestamp).unwrap_or(0.0);
        while let Some(&front_peak) = self.peaks.front() {
            if current_time - front_peak > HR_DETECTION_WINDOW {
                self.peaks.pop_front();
            } else {
                break;
            }
        }

        // Calculate heart rate from recent peaks
        if self.peaks.len() >= 3 {
            self.calculate_heart_rate()
        } else {
            self.last_hr
        }
    }

    fn detect_ppg_peaks(&self, data: &VecDeque<TimestampedValue>) -> Vec<f64> {
        let mut new_peaks = Vec::new();
        let window_size = 10; // Larger window for PPG
        
        // First, apply a simple moving average to smooth the signal
        let smoothed: Vec<(f64, f64)> = data.iter()
            .enumerate()
            .filter_map(|(i, tv)| {
                if i < window_size || i >= data.len() - window_size {
                    return None;
                }
                
                let sum: f64 = data.range(i-window_size..i+window_size+1)
                    .map(|v| v.value)
                    .sum();
                let avg = sum / (2 * window_size + 1) as f64;
                Some((tv.timestamp, avg))
            })
            .collect();

        if smoothed.len() < 20 {
            return new_peaks;
        }

        // Calculate derivative to find rising edges
        let mut derivatives = Vec::new();
        for i in 1..smoothed.len() {
            let derivative = smoothed[i].1 - smoothed[i-1].1;
            derivatives.push((smoothed[i].0, derivative));
        }

        // Find peaks using derivative zero-crossing method
        for i in 2..derivatives.len()-2 {
            let prev_deriv = derivatives[i-1].1;
            let curr_deriv = derivatives[i].1;
            let next_deriv = derivatives[i+1].1;
            
            // Look for zero crossing from positive to negative (peak)
            if prev_deriv > 0.0 && curr_deriv <= 0.0 && next_deriv < 0.0 {
                let timestamp = derivatives[i].0;
                
                // Check minimum distance from last peak
                if new_peaks.is_empty() || timestamp - new_peaks.last().unwrap() > PEAK_MIN_DISTANCE {
                    // Verify this is actually a significant peak
                    let peak_idx = smoothed.iter()
                        .position(|(t, _)| (t - timestamp).abs() < 0.01)
                        .unwrap_or(0);
                    
                    if peak_idx > 5 && peak_idx < smoothed.len() - 5 {
                        let peak_value = smoothed[peak_idx].1;
                        let left_min = smoothed[peak_idx-5..peak_idx].iter()
                            .map(|(_, v)| v)
                            .fold(f64::INFINITY, |a, &b| a.min(b));
                        let right_min = smoothed[peak_idx+1..peak_idx+6].iter()
                            .map(|(_, v)| v)
                            .fold(f64::INFINITY, |a, &b| a.min(b));
                        
                        // Peak must be significantly higher than surrounding valleys
                        if peak_value - left_min > 5.0 && peak_value - right_min > 5.0 {
                            new_peaks.push(timestamp);
                        }
                    }
                }
            }
        }

        new_peaks
    }

    fn detect_ecg_peaks(&self, data: &VecDeque<TimestampedValue>) -> Vec<f64> {
        let mut new_peaks = Vec::new();
        let window_size = 5;
        
        for i in window_size..data.len().saturating_sub(window_size) {
            let current = data[i].value;
            let left_avg: f64 = data.range(i-window_size..i).map(|v| v.value).sum::<f64>() / window_size as f64;
            let right_avg: f64 = data.range(i+1..i+1+window_size).map(|v| v.value).sum::<f64>() / window_size as f64;
            
            // Peak condition: current value is significantly higher than surrounding averages
            if current > left_avg + 50.0 && current > right_avg + 50.0 {
                let timestamp = data[i].timestamp;
                
                // Check minimum distance from last peak
                if new_peaks.is_empty() || timestamp - new_peaks.last().unwrap() > PEAK_MIN_DISTANCE {
                    new_peaks.push(timestamp);
                }
            }
        }

        new_peaks
    }

    fn calculate_heart_rate(&mut self) -> f64 {
        let recent_peaks: Vec<f64> = self.peaks.iter().cloned().collect();
        let mut intervals = Vec::new();
        
        for i in 1..recent_peaks.len() {
            let interval = recent_peaks[i] - recent_peaks[i-1];
            if interval > 0.0 {
                intervals.push(60.0 / interval); // Convert to BPM
            }
        }
        
        if !intervals.is_empty() {
            // Use median instead of mean for more robust heart rate calculation
            intervals.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median_hr = intervals[intervals.len() / 2];
            
            // Filter out unrealistic heart rates
            if median_hr >= HR_MIN_BPM && median_hr <= HR_MAX_BPM {
                // Calculate signal quality based on consistency of intervals
                let variance: f64 = intervals.iter()
                    .map(|hr| (hr - median_hr).powi(2))
                    .sum::<f64>() / intervals.len() as f64;
                
                self.signal_quality = 1.0 / (1.0 + variance / 100.0); // Normalize to 0-1
                self.last_hr = median_hr;
                return median_hr;
            }
        }

        self.last_hr
    }
}

struct Buffers {
    data: Vec<VecDeque<TimestampedValue>>,
    raw_history: Vec<VecDeque<f64>>,
    start_time: Option<f64>, // Reference time for relative timestamps
    hr_detectors: Vec<HeartRateDetector>,
    calculated_hr: f64,
    hr_source: String,
}

impl Buffers {
    fn new() -> Self {
        let mut data = Vec::new();
        let mut raw_history = Vec::new();
        let mut hr_detectors = Vec::new();
        
        for _ in 0..NUM_CHANNELS {
            data.push(VecDeque::new());
            raw_history.push(VecDeque::new());
            hr_detectors.push(HeartRateDetector::new());
        }
        
        Buffers { 
            data, 
            raw_history,
            start_time: None,
            hr_detectors,
            calculated_hr: 0.0,
            hr_source: String::from("---"),
        }
    }

    fn is_spike(&self, chan: usize, value: f64) -> bool {
        let history = &self.raw_history[chan];
        if history.len() < 10 {
            return false;
        }

        let sum: f64 = history.iter().sum();
        let mean = sum / history.len() as f64;
        
        let variance: f64 = history.iter()
            .map(|x| (x - mean).powi(2))
            .sum::<f64>() / history.len() as f64;
        let std_dev = variance.sqrt();

        let threshold = SPIKE_THRESHOLD_MULTIPLIER * std_dev;
        (value - mean).abs() > threshold && threshold > 1.0
    }

    fn median_filter(&self, chan: usize, value: f64) -> f64 {
        let history = &self.raw_history[chan];
        if history.len() < MEDIAN_WINDOW_SIZE {
            return value;
        }

        let mut window: Vec<f64> = history.iter()
            .rev()
            .take(MEDIAN_WINDOW_SIZE - 1)
            .copied()
            .collect();
        window.push(value);
        window.sort_by(|a, b| a.partial_cmp(b).unwrap());
        
        window[window.len() / 2]
    }

    fn push(&mut self, chan: usize, value: f64, timestamp_us: u64) {
        let timestamp_sec = timestamp_us as f64 / 1_000_000.0;
        
        // Set start time on first data point
        if self.start_time.is_none() {
            self.start_time = Some(timestamp_sec);
        }
        
        let relative_time = timestamp_sec - self.start_time.unwrap();

        if let Some(history) = self.raw_history.get_mut(chan) {
            if history.len() >= 50 {
                history.pop_front();
            }
            history.push_back(value);

            let filtered_value = if self.is_spike(chan, value) {
                self.median_filter(chan, value)
            } else {
                self.median_filter(chan, value)
            };

            if let Some(buf) = self.data.get_mut(chan) {
                // Remove old data points (keep last 30 seconds)
                while let Some(front) = buf.front() {
                    if relative_time - front.timestamp > 30.0 {
                        buf.pop_front();
                    } else {
                        break;
                    }
                }
                
                buf.push_back(TimestampedValue {
                    timestamp: relative_time,
                    value: filtered_value,
                });

                // Update heart rate detection for channels 0 and 1
                if chan <= 1 && buf.len() > 50 {
                    let _hr = self.hr_detectors[chan].detect_peaks(buf, chan);
                    
                    // Select the best heart rate source
                    self.update_best_hr();
                }
            }
        }
    }

    fn update_best_hr(&mut self) {
        let hr0 = self.hr_detectors[0].last_hr;
        let hr1 = self.hr_detectors[1].last_hr;
        let quality0 = self.hr_detectors[0].signal_quality;
        let quality1 = self.hr_detectors[1].signal_quality;

        // Choose the source with better signal quality
        if quality0 > quality1 && hr0 > 0.0 {
            self.calculated_hr = hr0;
            self.hr_source = String::from("PPG");
        } else if hr1 > 0.0 {
            self.calculated_hr = hr1;
            self.hr_source = String::from("ECG");
        } else if hr0 > 0.0 {
            self.calculated_hr = hr0;
            self.hr_source = String::from("PPG");
        }
    }

    fn as_points(&self, chan: usize) -> PlotPoints {
        self.data[chan]
            .iter()
            .map(|tv| [tv.timestamp, tv.value])
            .collect()
    }

    fn get_heart_rate(&self) -> (f64, String) {
        (self.calculated_hr, self.hr_source.clone())
    }
}

fn cobbs_encode(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut code_idx = 0;
    let mut code = 1;
    
    output.push(0); // Placeholder for first code byte
    
    for &byte in data {
        if byte == 0 {
            output[code_idx] = code;
            code_idx = output.len();
            output.push(0); // Placeholder for next code byte
            code = 1;
        } else {
            output.push(byte);
            code += 1;
            if code == 0xFF {
                output[code_idx] = code;
                code_idx = output.len();
                output.push(0); // Placeholder for next code byte
                code = 1;
            }
        }
    }
    
    output[code_idx] = code;
    output.push(0); // Terminating zero
    output
}

fn send_timestamp_sync(port: &mut Box<dyn serialport::SerialPort>) {
    let timestamp_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64;
    
    let mut packet = Vec::new();
    packet.push(0x03); // PACK_TYPE_TIME
    packet.extend_from_slice(&timestamp_us.to_le_bytes()); // 8 bytes timestamp
    packet.push(0); // time_type
    
    let encoded = cobbs_encode(&packet);
    let _ = port.write_all(&encoded);
}

/// COBBS decode
fn cobbs_decode(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let length = data[i];
        if length == 0 {
            break;
        }
        i += 1;
        if i + (length as usize) - 1 <= data.len() {
            output.extend_from_slice(&data[i..i + length as usize - 1]);
        }
        i += (length as usize) - 1;
        if length < 0xFF && i < data.len() {
            output.push(0);
        }
    }
    output
}

struct MyApp {
    buffers: Arc<Mutex<Buffers>>,
    last_vals: Arc<Mutex<[String; 4]>>,
}

impl App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let last = self.last_vals.lock().unwrap();
            let buf = self.buffers.lock().unwrap();
            let (hr, hr_source) = buf.get_heart_rate();
            
            ui.label(format!(
                "BP: {}   SpO2: {}   T1: {}   T2: {}",
                last[0], last[1], last[2], last[3]
            ));
            
            ui.label(format!(
                "Heart Rate: {:.0} BPM ({})",
                hr,
                if hr > 0.0 { hr_source.as_str() } else { "---" }
            ));
            
            drop(last);

            for chan in 0..NUM_CHANNELS {
                let line = Line::new(format!("chan{}", chan), buf.as_points(chan));
                Plot::new(format!("chan{}", chan))
                    .height(150.0)
                    .show(ui, |plot_ui| {
                        plot_ui.line(line);
                    });
            }
        });

        ctx.request_repaint();
    }
}

fn main() -> eframe::Result<()> {
    let buffers = Arc::new(Mutex::new(Buffers::new()));
    let last_vals = Arc::new(Mutex::new([
        String::from("---/--- (---)"),
        String::from("---"),
        String::from("---"),
        String::from("---"),
    ]));

    // Spawn serial reader
    {
        let buffers = Arc::clone(&buffers);
        let last_vals = Arc::clone(&last_vals);

        thread::spawn(move || {
            let mut port = serialport::new("COM5", 38400)
                .timeout(Duration::from_millis(10))
                .open()
                .expect("Failed to open serial port");

            send_timestamp_sync(&mut port);

            let mut packet: Vec<u8> = Vec::new();
            let mut buf = [0u8; 1];
            let mut last_sync = std::time::Instant::now();

            loop {
                if last_sync.elapsed() > Duration::from_secs(5) {
                    send_timestamp_sync(&mut port);
                    last_sync = std::time::Instant::now();
                }

                if let Ok(n) = port.read(&mut buf) {
                    if packet.len() > 256 {  // Reasonable limit
                        packet.clear();
                        continue;
                    }

                    if n == 0 {
                        continue;
                    }
                    let b = buf[0];
                    packet.push(b);
                    if b == 0 {
                        let decoded = cobbs_decode(&packet);
                        packet.clear();
                        if decoded.len() < 11 {
                            continue;
                        }
                        
                        // Extract timestamp from packet (bytes 1-8)
                        let timestamp_us = u64::from_le_bytes([
                            decoded[1], decoded[2], decoded[3], decoded[4],
                            decoded[5], decoded[6], decoded[7], decoded[8]
                        ]);
                        
                        let mut data = decoded[11..].to_vec();
                        if data.is_empty() {
                            continue;
                        }

                        let mut bufs = buffers.lock().unwrap();
                        let mut lv = last_vals.lock().unwrap();

                        match data[0] {
                            100 => {
                                let mut payload = data[1..].to_vec();
                                if payload.len() % 2 == 1 {
                                    payload.pop();
                                }
                                let sample_interval_us = (1_000_000.0 / PPG_SAMPLE_RATE_HZ) as u64;
                                for (i, chunk) in payload.chunks(4).enumerate() {
                                    if let Some(&v0) = chunk.get(0) {
                                        let sample_timestamp = timestamp_us + (i as u64 * sample_interval_us);
                                        bufs.push(0, v0 as f64, sample_timestamp);
                                    }
                                }
                            }
                            20 => {
                                let payload = &data[1..data.len().saturating_sub(1)];
                                let sample_interval_us = (1_000_000.0 / ECG_SAMPLE_RATE_HZ) as u64;
                                let mut sample_count = 0;
                                for chunk in payload.chunks(2) {
                                    if chunk.len() == 2 && sample_count % 2 == 0 {
                                        let val = ((chunk[1] as i32) << 8 | chunk[0] as i32) as f64;
                                        if val > 400.0 && val < 5000.0 {
                                            let sample_timestamp = timestamp_us + (sample_count as u64 * sample_interval_us / 2);
                                            bufs.push(1, val, sample_timestamp);
                                        }
                                    }
                                    sample_count += 1;
                                }
                            }
                            84 => {
                                if data.len() > 9 {
                                    let payload = &data[9..data.len() - 1];
                                    let sample_interval_us = (1_000_000.0 / BP_SAMPLE_RATE_HZ) as u64;
                                    for (i, chunk) in payload.chunks(2).enumerate() {
                                        if chunk.len() == 2 {
                                            let val = ((chunk[1] as i32) << 8 | (chunk[0] as i32) << 0) as f64;
                                            let sample_timestamp = timestamp_us + (i as u64 * sample_interval_us);
                                            bufs.push(2, val, sample_timestamp);
                                        }
                                    }
                                }
                            }
                            5 => {
                                // Handle non-waveform data (no timestamp needed for display values)
                                if data.len() >= 7 && data[1] == 42 {
                                    lv[0] = format!("{}/{} ({})", data[2], data[4], data[6]);
                                }
                                if data.len() >= 3 && data[1] == 48 {
                                    lv[1] = if data[2] != 0 {
                                        data[2].to_string()
                                    } else {
                                        String::from("---")
                                    };
                                }
                                if data.len() >= 4 && data[1] == 46 {
                                    let temp = ((data[3] as u16) << 8 | data[2] as u16) as f32 / 30.0;
                                    lv[2] = format!("{:.1} C", temp);
                                }
                                if data.len() >= 4 && data[1] == 47 {
                                    let temp = ((data[3] as u16) << 8 | data[2] as u16) as f32 / 30.0;
                                    lv[3] = format!("{:.1} C", temp);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        });
    }

    let app = MyApp { buffers, last_vals };
    let native_opts = eframe::NativeOptions::default();
    eframe::run_native(
        "Vitals GUI",
        native_opts,
        Box::new(|_cc| Ok(Box::new(app))),
    )
}
#include <stdio.h>
#include "pico/stdlib.h"
#include "hardware/pio.h"
#include "hardware/clocks.h"
#include "uart9_rx.pio.h"
#include "pico/stdlib.h"
#include "hardware/dma.h"
#include <string.h>
#include "pico/multicore.h"
#include "hardware/vreg.h"

static int dma_chan = -1;
static uint8_t dma_buf[2][512];  // Double buffer
static volatile bool dma_busy = false;
static uint8_t current_dma_buf = 0;
static uint16_t dma_buf_pos = 0;

// Pin 0 is on the TX of digital board (digital -> analog)
// Pin 1 is on the RX of digital board (analog -> digital)
// Pin 4 and 5 are connected to the serial chip
#define TX_PIN 0
#define RX_PIN 1
#define BAUD   168000

#define HEADER_LENGTH 11

#define USE_USB_SERIAL 0
#define SERIAL_UART_ID uart1
#define SERIAL_RX_PIN 4
#define SERIAL_TX_PIN 5
#define SERIAL_BAUD 38400

#define CLOCK_HZ 360000000

#define PACK_TYPE_DATA 0x01
#define PACK_TYPE_CMD  0x02
#define PACK_TYPE_TIME 0x03

void cobbs_decode(uint8_t *data, uint8_t length, uint8_t *output, uint8_t *out_length) {
    int out_idx = 0;
    int idx = 0;

    while (idx < length) {
        uint8_t code = data[idx++];
        for (int i = 1; i < code; i++) {
            if (idx >= length) break;
            output[out_idx++] = data[idx++];
        }
        if (code != 0xFF && idx < length) {
            output[out_idx++] = 0;
        }
    }
    *out_length = out_idx;
}

void serial_init() {
    #if USE_USB_SERIAL
        stdio_init_all();
    #else
        uart_init(SERIAL_UART_ID, SERIAL_BAUD);
        gpio_set_function(SERIAL_TX_PIN, GPIO_FUNC_UART);
        gpio_set_function(SERIAL_RX_PIN, GPIO_FUNC_UART);
        uart_set_hw_flow(SERIAL_UART_ID, false, false);
        uart_set_format(SERIAL_UART_ID, 8, 1, UART_PARITY_NONE);
        uart_set_fifo_enabled(SERIAL_UART_ID, true);
        
        // Setup DMA for UART TX
        dma_chan = dma_claim_unused_channel(true);
        dma_channel_config dma_config = dma_channel_get_default_config(dma_chan);
        channel_config_set_transfer_data_size(&dma_config, DMA_SIZE_8);
        channel_config_set_dreq(&dma_config, uart_get_dreq(SERIAL_UART_ID, true));
        dma_channel_configure(dma_chan, &dma_config, &uart_get_hw(SERIAL_UART_ID)->dr, NULL, 0, false);
    #endif
    }
    
    void serial_write_fast(const void *data, size_t size) {
    #if USE_USB_SERIAL
        fwrite(data, 1, size, stdout);
    #else
        // Copy to DMA buffer if there's space
        if (dma_buf_pos + size < 512) {
            memcpy(&dma_buf[current_dma_buf][dma_buf_pos], data, size);
            dma_buf_pos += size;
        } else {
            // Flush current buffer via DMA if not busy
            if (!dma_busy && dma_buf_pos > 0) {
                dma_busy = true;
                dma_channel_set_read_addr(dma_chan, dma_buf[current_dma_buf], false);
                dma_channel_set_trans_count(dma_chan, dma_buf_pos, true);
                
                // Switch to other buffer
                current_dma_buf = 1 - current_dma_buf;
                dma_buf_pos = 0;
                
                // Copy new data to fresh buffer
                if (size < 512) {
                    memcpy(&dma_buf[current_dma_buf][0], data, size);
                    dma_buf_pos = size;
                }
            }
            // If DMA still busy, use blocking write as fallback
            else {
                uart_write_blocking(SERIAL_UART_ID, (const uint8_t*)data, size);
            }
        }
        
        // Check if DMA transfer completed
        if (dma_busy && !dma_channel_is_busy(dma_chan)) {
            dma_busy = false;
        }
    #endif
    }

int serial_read_timeout(uint32_t timeout_us) {
#if USE_USB_SERIAL
    return getchar_timeout_us(timeout_us);
#else
    if (uart_is_readable_within_us(SERIAL_UART_ID, timeout_us)) {
        return uart_getc(SERIAL_UART_ID);
    }
    return PICO_ERROR_TIMEOUT;
#endif
}

void process_command(uint8_t *cmd_data, uint8_t cmd_len, uint64_t *time_offset, uint8_t *time_type) {
    if (cmd_len < 1) return;
    
    uint8_t cmd = cmd_data[0];
    
    switch (cmd) {
        case PACK_TYPE_TIME:
            if (cmd_len >= 10) { // 1 byte cmd + 8 bytes timestamp + 1 byte time_type
                *time_offset = *((uint64_t*)(cmd_data + 1));
                *time_type = cmd_data[9];
                
                // Calculate offset adjustment based on current time
                uint64_t current_time = time_us_64();
                *time_offset = *time_offset - current_time;
            }
            break;
            
        case PACK_TYPE_CMD:
            if (cmd_len >= 2) { // 1 byte cmd + at least 1 byte data
                // TODO: Implement sending data out of TX_PIN using PIO
                // this is maybe actually a bad idea because i don't have a good
                // way to prevent collisions with data from the digital board
            }
            break;
    }
}

// this is for time sync packets from the host device
void check_serial_commands_fast(uint64_t *time_offset, uint8_t *time_type) {
    static uint8_t serial_buffer[256];
    static uint8_t serial_length = 0;
    static bool in_packet = false;
    
    // Read multiple bytes at once if available
    while (uart_is_readable(SERIAL_UART_ID)) {
        int c = uart_getc(SERIAL_UART_ID);
        
        if (!in_packet && c != 0) {
            in_packet = true;
            serial_length = 0;
        }
        
        if (in_packet) {
            serial_buffer[serial_length++] = (uint8_t)c;
            
            if (c == 0 || serial_length >= 256) {
                if (c == 0 && serial_length > 1) {
                    uint8_t decoded[256];
                    uint8_t decoded_length;
                    cobbs_decode(serial_buffer, serial_length, decoded, &decoded_length);
                    
                    if (decoded_length > 0) {
                        process_command(decoded, decoded_length, time_offset, time_type);
                    }
                }
                in_packet = false;
                serial_length = 0;
                break; // Process one packet per call
            }
        }
    }
}

// these packets are the waveforms for ecg, spo2, and blood pressure and the info packets
static inline bool is_valid_packet_type(uint8_t type) {
    return (type == 100 || type == 20 || type == 84 || type == 5);
}

int main() {
    vreg_set_voltage(VREG_VOLTAGE_1_20);

    // sleep to allow the voltage to stabilize before overclocking
    sleep_ms(1000);
    set_sys_clock_khz(CLOCK_HZ/1000, true);
    
    serial_init();
    uint64_t time_offset = 0;
    uint8_t time_type = 0;
    
    PIO pio = pio0;
    uint offset_rx = pio_add_program(pio, &uart9_rx_program);
    uint sm_rx = pio_claim_unused_sm(pio, true);
    
    pio_sm_config c = uart9_rx_program_get_default_config(offset_rx);
    sm_config_set_in_pins(&c, RX_PIN);
    sm_config_set_fifo_join(&c, PIO_FIFO_JOIN_RX);

    float clkdiv = (float)CLOCK_HZ / (BAUD * 8.0f);
    
    // Round to nearest 1/256th for PIO precision
    uint16_t div_int = (uint16_t)clkdiv;
    uint8_t div_frac = (uint8_t)((clkdiv - div_int) * 256);
    
    sm_config_set_clkdiv_int_frac(&c, div_int, div_frac);

    pio_sm_init(pio, sm_rx, offset_rx, &c);
    pio_sm_set_enabled(pio, sm_rx, true);
    
    uint8_t send_buf[256];
    uint8_t out_buf[256];
    uint8_t length = 0;
    
    // Pre-calculate header
    send_buf[0] = 0x01; // Packet type
    send_buf[9] = 0;    // Will be updated with time_type
    
    uint32_t serial_check_counter = 0;
    
    while (true) {
        check_serial_commands_fast(&time_offset, &time_type);
        
        // get the PIO data from the analog board
        while (!pio_sm_is_rx_fifo_empty(pio, sm_rx)) {
            uint32_t raw = pio_sm_get(pio, sm_rx);
            uint16_t n = (raw >> 23) & 0x1FF;
            
            if (__builtin_expect(length > 255, 0)) length = 0;
            
            if (length > 0 && (n & 0x100)) {
                if (__builtin_expect(length > 1, 1)) {
                    uint8_t data_type = send_buf[HEADER_LENGTH];
                    if (__builtin_expect(is_valid_packet_type(data_type), 1)) {
                        *((uint64_t*)(send_buf + 1)) = time_offset + time_us_64();
                        send_buf[10] = length;
                        
                        // cobs encoding
                        uint8_t *out_ptr = out_buf;
                        uint8_t *code_ptr = out_ptr++;
                        uint8_t code = 1;
                        uint8_t total_len = HEADER_LENGTH + length;
                        
                        for (uint8_t i = 0; i < total_len; i++) {
                            if (send_buf[i] == 0) {
                                *code_ptr = code;
                                code = 1;
                                code_ptr = out_ptr++;
                            } else {
                                *out_ptr++ = send_buf[i];
                                if (++code == 0xFF) {
                                    *code_ptr = code;
                                    code = 1;
                                    code_ptr = out_ptr++;
                                }
                            }
                        }
                        *code_ptr = code;
                        *out_ptr++ = 0;
                        
                        serial_write_fast(out_buf, out_ptr - out_buf);
                    }
                }
                length = 1;
                send_buf[HEADER_LENGTH] = n & 0xFF;
            } else {
                send_buf[HEADER_LENGTH + length++] = n & 0xFF;
            }
        }
    }
}

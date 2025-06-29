import serial
import time

# === CONFIGURATION ===
PORT = 'COM3'        # Change this to your actual port, e.g., '/dev/ttyUSB0' on Linux
BAUDRATE = 9600      # Set your baud rate
OUTPUT_FILE = 'output.bin'

# === MAIN ===
try:
    ser = serial.Serial(PORT, BAUDRATE, timeout=1)
    print(f"Opened {PORT} at {BAUDRATE} baud. Saving to {OUTPUT_FILE}...")

    with open(OUTPUT_FILE, 'wb') as f:
        while True:
            data = ser.read(1024)  # Read up to 1024 bytes at a time
            if data:
                f.write(data)
                f.flush()
except KeyboardInterrupt:
    print("\nInterrupted by user. Closing...")
except serial.SerialException as e:
    print(f"Serial error: {e}")
finally:
    if 'ser' in locals() and ser.is_open:
        ser.close()
    print("Serial port closed.")

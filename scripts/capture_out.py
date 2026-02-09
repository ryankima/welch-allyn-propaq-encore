'''
This is intended to capture the output test binary data from the welch allyn
propaq encore. Run this script first, enter the net test under the service menu
of the encore, run the output test, and once it's done, ctrl+c this script.
'''

import serial

port = 'COM3'  # change this to your serial port
baud = 38400

ser = serial.Serial(port, baud, timeout=1)
f = open('capture_out.bin', 'wb')
print(f'Opened {port} at {baud} baud')
try:
    while True:
        data = ser.read(1)
        if data:
            f.write(data)
finally:
    f.close()
    ser.close()
    print(f'Closed {port}')

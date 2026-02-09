'''
This is the bare minimum script to pass loop test on the Welch Allyn Propaq Encore.
Run this script first, then run the loop test under the service menu of the Encore.
If it does not pass, check your wire connections again.
'''

import serial

port = 'COM3'  # change this to your serial port
baud = 38400

ser = serial.Serial(port, baud, timeout=1)
print(f'Opened {port} at {baud} baud')
try:
    while True:
        data = ser.read(1)
        print(f'Received: {data.hex()}')
        if data:
            ser.write(data)  # echo back the received data
finally:
    ser.close()
    print(f'Closed {port}')

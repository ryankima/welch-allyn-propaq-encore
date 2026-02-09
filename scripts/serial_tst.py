import serial, time, math

ser = serial.Serial("COM3", 38400, timeout=0)

def send(seq):
    ser.write(bytes(seq))
    time.sleep(0.05)

def test():
     for sync in range(0xFB, 0x100):  # SYNC byte ≥ 0xFB
        inv = sync ^ 0xFF

        for length in range(1, 14):  # Max 13 byte payload (16 total)
            payload = [0xAA] * length  # Can tweak this pattern later

            pkt = bytes([sync, inv, length] + payload)
            ser.reset_input_buffer()
            ser.reset_output_buffer()

            ser.write(pkt)
            print(f"Sent: {[hex(b) for b in pkt]}")
            time.sleep(0.1)
            resp = ser.read_all()
            if resp:
                print(f"Response: {[hex(b) for b in resp]}")

ser.write(b"\xff\xaa\x55\x00")

i = 1
while True:
    data = i.to_bytes((math.floor(math.log2(i))//8)+1, 'big')  # Convert to bytes
    ser.write(data)
    print(f"Sent: {data.hex()}")
    #time.sleep(0.00001)
    resp = ser.read_all()
    if resp:
        print(f"Response: {[hex(b) for b in resp]}")
        ser.write(resp)  # Echo back the response
    i += 1

ser.close()

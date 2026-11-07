#!/usr/bin/env python3
"""
simplex_audio_bridge.py - Real-time 8 kHz audio bridge with half-duplex / simplex simulation.

Modes:
  --duplex   : Independent concurrent full-duplex channels (A->B and B->A simultaneously).
  --simplex  : Shared half-duplex RF channel (default). If both stations transmit simultaneously,
               an RF collision occurs and noise is injected, corrupting packets.
               Transmitting stations cannot receive their own or remote traffic while keyed up.
"""
import os
import sys
import time
import signal
import argparse

def main():
    parser = argparse.ArgumentParser(description="Real-time 8 kHz audio bridge for Mercury TNC simulation")
    parser.add_argument("a_tx", help="FIFO path for Station A transmission")
    parser.add_argument("b_rx", help="FIFO path for Station B reception")
    parser.add_argument("b_tx", help="FIFO path for Station B transmission")
    parser.add_argument("a_rx", help="FIFO path for Station A reception")
    parser.add_argument("--simplex", action="store_true", default=True, help="Simulate shared simplex RF channel with collision detection (default: true)")
    parser.add_argument("--duplex", dest="simplex", action="store_false", help="Run in full-duplex mode without collision")
    args = parser.parse_args()

    stop_bridge = False

    def handle_signal(sig, frame):
        nonlocal stop_bridge
        stop_bridge = True

    signal.signal(signal.SIGINT, handle_signal)
    signal.signal(signal.SIGTERM, handle_signal)

    # In Linux, opening FIFOs in nonblocking mode
    print(f"[*] Opening audio FIFOs (simplex mode: {args.simplex})...")
    # Open readers first (nonblocking)
    a_tx_fd = os.open(args.a_tx, os.O_RDONLY | os.O_NONBLOCK)
    b_tx_fd = os.open(args.b_tx, os.O_RDONLY | os.O_NONBLOCK)
    # Open writers
    b_rx_fd = os.open(args.b_rx, os.O_WRONLY)
    a_rx_fd = os.open(args.a_rx, os.O_WRONLY)

    CHUNK_SAMPLES = 160               # 20 ms at 8000 Hz
    CHUNK_BYTES = CHUNK_SAMPLES * 4   # 640 bytes (PCM s32le)
    silence = b'\x00' * CHUNK_BYTES
    # Pseudo-random RF collision noise buffer
    noise = bytes((i * 107 + 91) & 0xFF for i in range(CHUNK_BYTES))

    next_tick = time.time()
    collision_count = 0

    try:
        while not stop_bridge:
            # Read Station A TX
            try:
                buf_a = os.read(a_tx_fd, CHUNK_BYTES)
            except BlockingIOError:
                buf_a = b''
            except Exception:
                break

            # Read Station B TX
            try:
                buf_b = os.read(b_tx_fd, CHUNK_BYTES)
            except BlockingIOError:
                buf_b = b''
            except Exception:
                break

            chunk_a = buf_a if len(buf_a) == CHUNK_BYTES else buf_a + silence[:CHUNK_BYTES - len(buf_a)]
            chunk_b = buf_b if len(buf_b) == CHUNK_BYTES else buf_b + silence[:CHUNK_BYTES - len(buf_b)]

            a_active = any(b != 0 for b in chunk_a)
            b_active = any(b != 0 for b in chunk_b)

            if args.simplex:
                if a_active and b_active:
                    # Simplex collision! Both stations transmitting on the same RF frequency
                    collision_count += 1
                    if collision_count % 50 == 1:
                        print("[!] Simplex RF collision detected on channel (both stations transmitting)!")
                    out_b_rx = noise
                    out_a_rx = noise
                elif a_active:
                    # Station A transmitting -> delivered to B, A is muted (half-duplex)
                    out_b_rx = chunk_a
                    out_a_rx = silence
                elif b_active:
                    # Station B transmitting -> delivered to A, B is muted (half-duplex)
                    out_b_rx = silence
                    out_a_rx = chunk_b
                else:
                    out_b_rx = silence
                    out_a_rx = silence
            else:
                out_b_rx = chunk_a
                out_a_rx = chunk_b

            try:
                os.write(b_rx_fd, out_b_rx)
                os.write(a_rx_fd, out_a_rx)
            except Exception:
                break

            next_tick += 0.020
            sleep_dur = next_tick - time.time()
            if sleep_dur > 0:
                time.sleep(sleep_dur)
            else:
                next_tick = time.time()
    finally:
        for fd in (a_tx_fd, b_tx_fd, b_rx_fd, a_rx_fd):
            try:
                os.close(fd)
            except Exception:
                pass
        print(f"[*] Audio bridge exited cleanly (collisions recorded: {collision_count}).")

if __name__ == '__main__':
    main()

# pico-dac3

`pico-dac3` is USB DAC firmware written in Rust for the Raspberry Pi RP2040
and RP2350 microcontrollers. It is a Rust rewrite of
[pico-dac2](https://github.com/td2sk/pico-dac2).

> [!NOTE]
> RP2040 has been tested on hardware. RP2350 Arm and RP2350 RISC-V support is
> currently a work in progress and has only been build-tested.

## Features

- USB Audio Class 2.0 stereo output
- 44.1, 48, 88.2, and 96 kHz sample rates
- 16, 24, and 32 bit PCM
- Asynchronous isochronous transfer with an explicit feedback endpoint
- Master, left, and right channel volume and mute controls
- Vendor-defined 16-byte HID IN/OUT interface
- PIO and DMA based I2S output
- RP2040, RP2350 Arm, and RP2350 RISC-V targets

## Default I2S Pins

| Signal | GPIO |
| --- | ---: |
| BCLK | 20 |
| LRCLK | 21 |
| DATA | 22 |

The current I2S implementation is intended for an external I2S DAC such as
the PCM5102A. To change the pins, update the GPIO assignments and PIO pin
numbers in `src/main.rs`.

## Building

Install Rust and the Raspberry Pi Pico VS Code extension, then clone this
repository including its patched `usb-device` submodule:

```sh
git clone --recurse-submodules https://github.com/td2sk/pico-dac3.git
cd pico-dac3
```

Select the target through the Pico extension or build directly with Cargo:

```sh
# RP2040
cargo build --release --target thumbv6m-none-eabi

# RP2350 Arm
cargo build --release --target thumbv8m.main-none-eabihf

# RP2350 RISC-V
cargo build --release --target riscv32imac-unknown-none-elf
```

The Pico extension can build, flash, and debug the firmware. Alternatively,
the generated ELF can be loaded with `picotool`.

For an existing clone, initialize the dependency with:

```sh
git submodule update --init --recursive
```

## USB Identification

- Vendor ID: `0xcafe`
- Product ID: `0xbabe`

## License

MIT. See [LICENSE](LICENSE).

# qaf

![Build](https://github.com/aggron/qaf/workflows/Build/badge.svg)
![Release](https://github.com/aggron/qaf/workflows/Release/badge.svg)

A macOS menubar application for controlling KEF speakers via their network API.

## Features

- Control KEF speakers from your Mac's menubar
- Switch between input sources (USB, WiFi, Bluetooth, Optical, TV)
- Power on/off control
- Automatic speaker discovery via mDNS
- Native macOS app built with Rust

## Installation

### Download Binary

Download the latest release for Apple Silicon Macs:

```bash
# Download and install
curl -L https://github.com/dvdplm/qaf/releases/latest/download/qaf-apple-silicon.tar.gz | tar xz
chmod +x qaf
qaf
```

### Build from Source

#### Prerequisites

- Rust 1.70 or later
- macOS 11.0 or later
- Xcode Command Line Tools

#### Building

```bash
# Clone the repository
git clone https://github.com/dvdplm/qaf.git
cd qaf

# Build
cargo build --release

# Install to ~/bin (or wherever you like to keep local executables)
sudo cp target/release/qaf ~/bin
```

## Usage

Run `qaf` from the terminal:

```bash
qaf
```

The app will appear in your menubar and automatically discover KEF speakers on your network.

## Supported Speakers

Tested with:
- KEF LSX II

Possibly works with other KEF speakers that support the network control API, e.g. KEF LS50 Wireless II or KEF LS60 Wireless.

## License

This project is licensed under the MIT License.

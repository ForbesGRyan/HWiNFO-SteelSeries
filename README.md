# HWiNFO-SteelSeries OLED

Display real-time hardware monitoring data from HWiNFO on your SteelSeries OLED screens. Lightweight, customizable, and easy to configure.

![HWiNFO-SteelSeries OLED Screenshot](/assets/hwinfo-steelseries-oled.png)

## Features

- **Low Resource Usage**: Uses only ~4MB of RAM and minimal CPU
- **Multiple Display Modes**:
  - **Summary Mode**: Pre-configured CPU/GPU/MEM layouts (Vertical or Horizontal)
  - **Custom Mode**: Choose any sensors from HWiNFO with full control
- **Multi-Page Support**: Cycle through multiple pages of sensor data
- **Real-Time Updates**: Direct access to HWiNFO's shared memory for instant updates
- **Easy Configuration**: Simple INI-based configuration file
- **System Tray Integration**: Runs quietly in the background

## Supported Devices

Any SteelSeries device with an OLED screen supported by SteelSeries GG:
- Arctis Pro Wireless
- Arctis Nova Pro Wireless
- Apex Pro
- Apex Pro TKL
- GameDAC

## Requirements

### Software

- **Windows 10/11** (64-bit)
- **[HWiNFO](https://www.hwinfo.com/)** (v7.0 or later recommended)
- **[SteelSeries GG](https://steelseries.com/gg)** (latest version)

### Hardware

- A SteelSeries device with OLED screen support

## Installation

### Option 1: Download Pre-built Binary

1. Download the latest `hwinfo-steelseries-oled.exe` from the [Releases](../../releases) page
2. Place it in a folder of your choice
3. Run the executable - it will guide you through initial setup

### Option 2: Build from Source

See [Building from Source](#building-from-source) section below.

## Setup

### 1. Configure HWiNFO

1. Launch HWiNFO and click **Start with Sensors** checked

   ![HWiNFO Sensors](/assets/hwinfo-sensors.png)

2. Open HWiNFO Settings (click the gear icon in the Sensors window)
3. Enable **Shared Memory Support** under the "General" section

   ![HWiNFO Shared Memory](/assets/hwinfo-shared-memory.png)

4. Keep HWiNFO running in the background

### 2. Launch SteelSeries GG

Make sure SteelSeries GG is running and your device is connected.

### 3. Run HWiNFO-SteelSeries OLED

Run `hwinfo-steelseries-oled.exe`. On first launch:

1. You'll be prompted to choose a display style:
   - **Summary Vertical**: 3-column layout (CPU/GPU/MEM)
   - **Summary Horizontal**: 3-row layout
   - **Custom**: Choose your own sensors

2. For **Summary** modes with multiple GPUs, you'll be asked to select which GPU to monitor

3. For **Custom** mode, you'll configure:
   - Number of lines (2-3)
   - Sensors per line (1-3)
   - Which specific sensors to display

4. Configuration is saved to `conf.ini` in the same directory

The application will start displaying data on your SteelSeries OLED screen.

## Configuration

### Summary Modes

#### Vertical Layout
```
CPU   GPU   MEM
55°   45°   8.6G
10%   0.0%  32G
```

#### Horizontal Layout
```
CPU  45°  10.0%
GPU  35°  0.0%
MEM  10G  33.3%
```

### Summary Mode Configuration

```ini
[Main]
style=Vertical    # or Horizontal
decimal=true      # Show decimal places (true/false)
gpu=GPU [#0]: NVIDIA GeForce RTX 3090  # Specific GPU (if multiple GPUs)
```

### Custom Mode Configuration

Custom mode allows you to display any sensor from HWiNFO with full control over labels and units.

```ini
[Main]
style=Custom
sensors_per_line=3  # 1-3 sensors per line
pages=2             # Number of pages
page_time=10        # Seconds between page switches
decimal=false       # Show decimal places

[PAGE1.Sensors]
sensor_0="RTSS;Framerate"
label_0="FPS"
unit_0=""

sensor_1="GPU [#0]: NVIDIA GeForce RTX 3090;GPU Temperature"
label_1="GPU"
unit_1="°"

sensor_2="GPU [#0]: NVIDIA GeForce RTX 3090;GPU Core Load"
label_2=""
unit_2="%"

sensor_3="CPU [#0]: AMD Ryzen 9 7950X3D;CPU (Tctl/Tdie)"
label_3="CPU"
unit_3="°"

sensor_4="CPU [#0]: AMD Ryzen 9 7950X3D;Total CPU Usage"
label_4=""
unit_4="%"

sensor_5="CPU [#0]: AMD Ryzen 9 7950X3D;CPU Package Power"
label_5=""
unit_5="W"

[PAGE2.Sensors]
sensor_0="System: ASUS;Physical Memory Used"
label_0="RAM"
unit_0="G"
convert_0="MB/GB"

sensor_1="System: ASUS;Physical Memory Available"
label_1=""
unit_1="G"
convert_1="MB/GB"

sensor_2="System: ASUS;Physical Memory Load"
label_2=""
unit_2="%"

sensor_3="Network: Intel Ethernet Controller I225-V;Current UP rate"
label_3="NET ▲"
unit_3="k/s"

sensor_4="Network: Intel Ethernet Controller I225-V;Current DL rate"
label_4="NET ▼"
unit_4="k/s"
```

#### Custom Mode Output Examples

**Page 1:**
```
FPS 144 GPU 45° 12%
CPU 55° 08% 89W
```

**Page 2:**
```
RAM 15G 48G 23%
NET ▲ 125k/s
NET ▼ 1.2M/s
```

### Sensor Format

Sensors are specified as: `"Sensor Category;Reading Name"`

To find sensor names:
1. Look at HWiNFO Sensors window
2. The category is the main sensor name (e.g., "GPU [#0]: NVIDIA GeForce RTX 3090")
3. The reading is the specific metric (e.g., "GPU Temperature")

### Special Sensors

- **CLOCK**: Displays current time
- **BLANK**: Empty sensor (useful for spacing)
- **RTSS**: Framerate from RivaTuner Statistics Server (if installed)

### Configuration Options Reference

| Option | Description | Default | Values |
|--------|-------------|---------|--------|
| `style` | Display mode | - | `Vertical`, `Horizontal`, `Custom` |
| `decimal` | Show decimal places | `false` | `true`, `false` |
| `gpu` | Specific GPU to monitor | First GPU | Full GPU sensor name |
| `sensors_per_line` | Sensors per line (Custom mode) | `1` | `1`, `2`, `3` |
| `pages` | Number of pages (Custom mode) | `1` | `1`-`10` |
| `page_time` | Seconds between pages | `5` | `0`-`60` |

## Troubleshooting

### "Disconnected from HWiNFO" Error

**Problem**: The OLED displays "Disconnected FROM HWiNFO"

**Solutions**:
1. Verify HWiNFO is running with the Sensors window open
2. Ensure "Shared Memory Support" is enabled in HWiNFO settings
3. Restart both HWiNFO and this application
4. Try running HWiNFO as Administrator

### OLED Screen Not Updating

**Problem**: The screen doesn't show any data or doesn't update

**Solutions**:
1. Verify SteelSeries GG is running
2. Check that your device is properly connected
3. Try restarting SteelSeries GG
4. Verify the device works with other SteelSeries Engine apps

### Sensor Not Found Error

**Problem**: Application can't find a specific sensor

**Solutions**:
1. Check the sensor name exactly matches what's shown in HWiNFO
2. Some sensors only appear when the hardware is active
3. Run the config wizard again to see available sensors
4. Delete `conf.ini` and reconfigure from scratch

### Configuration File Errors

**Problem**: Application won't start or config errors appear

**Solutions**:
1. Delete `conf.ini` to regenerate from scratch
2. Check for typos in sensor names (copy-paste from HWiNFO)
3. Ensure all required fields are present
4. Verify INI file format is correct (no missing brackets)

### High CPU/GPU Usage

**Problem**: Application uses more resources than expected

**Solutions**:
1. Increase `page_time` to reduce update frequency
2. Reduce number of custom sensors
3. Check for other applications polling HWiNFO
4. Verify HWiNFO isn't polling too frequently

## Building from Source

### Prerequisites

- [Rust](https://rustup.rs/) (latest stable version)
- Windows 10/11 SDK
- Git

### Build Steps

1. Clone the repository:
   ```bash
   git clone https://github.com/yourusername/HWiNFO-SteelSeries.git
   cd HWiNFO-SteelSeries
   ```

2. Build the project:
   ```bash
   cargo build --release
   ```

3. The executable will be in `target/release/hwinfo-steelseries-oled.exe`

### Development Build

For development with debug symbols:
```bash
cargo build
cargo run
```

### Running Tests

```bash
cargo test
```

## Project Structure

```
HWiNFO-SteelSeries/
├── src/
│   ├── lib.rs           # HWiNFO shared memory interface
│   ├── main.rs          # Application entry point
│   ├── connect.rs       # Connection handlers
│   ├── settings.rs      # Configuration wizard
│   ├── steelseries.rs   # SteelSeries GG API
│   ├── utils.rs         # Helper functions
│   ├── console_utils.rs # Console window management
│   └── consts.rs        # Constants
├── assets/              # Images and icons
├── conf.ini             # User configuration (generated)
└── Cargo.toml           # Rust project manifest
```

## Contributing

Contributions are welcome! Here's how you can help:

### Reporting Bugs

1. Check if the issue already exists in [Issues](../../issues)
2. Create a new issue with:
   - Clear description of the problem
   - Steps to reproduce
   - Your system configuration
   - Screenshots if applicable

### Feature Requests

Open an issue with the `enhancement` label and describe:
- The feature you'd like to see
- Why it would be useful
- How you envision it working

### Pull Requests

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Run tests and ensure code builds
5. Commit your changes (`git commit -m 'Add amazing feature'`)
6. Push to your branch (`git push origin feature/amazing-feature`)
7. Open a Pull Request

### Code Style

- Follow Rust standard formatting (`cargo fmt`)
- Run Clippy and address warnings (`cargo clippy`)
- Add tests for new functionality
- Update documentation as needed

## Acknowledgments

- **HWiNFO**: For providing the shared memory interface
- **SteelSeries**: For the GG platform and OLED support
- **Rust Community**: For excellent tooling and libraries

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Support

- **Issues**: [GitHub Issues](../../issues)
- **Discussions**: [GitHub Discussions](../../discussions)

---

Made with ❤️ for the hardware monitoring community

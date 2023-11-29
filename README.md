# HWiNFO-SteelSeriesOLED
Pulls info from HWiNFO Shared memory support and pushes it to Steel Series supported OLED screens

![hwinfo-steelseries-oled.png](/assets/hwinfo-steelseries-oled.png)

Currently pulling info from the following sensors:

```
CPU   | GPU   | MEM
Temp  | Temp  | Used
Usage | Usage | Free
```

CPU Usage = "Total CPU Usage"

CPU Temp  = "CPU (Tctl/Tdie)"

GPU Usage = "GPU Core Load"

GPU Temp  = "GPU Temperature"

MEM Used  = "Physical Memory Used"

MEM Free  = "Physical Memory Available"


## Requirements
**HWiNFO**
https://www.hwinfo.com/

**SteelSeries GG**
https://steelseries.com/gg


## Steps for running:
- Make sure SteelSeries GG is running
- Make sure HWiNFO is running
  - Enable "Shared Memory Support" in HWiNFO settings
  ![hwinfo-shared-memory.png](/assets/hwinfo-shared-memory.png)
- Run the `hwinfo-steelseries-oled.exe` file

# HWiNFO-SteelSeriesOLED
Pulls info from HWiNFO Shared memory support and pushes it to SteelSeries supported OLED screens.
Uses very little CPU and RAM (~4MB).

![hwinfo-steelseries-oled.png](/assets/hwinfo-steelseries-oled.png)

2 Summary templates are provided or you choose custom sensors with the `conf.ini` file.

`Vertical`
```
CPU   | GPU   | MEM
Temp  | Temp  | Used
Usage | Usage | Free
```

`Horizontal`
```
CPU  | Temp  | Usage
GPU  | Temp  | Usage 
MEM  | Used  | Free
```

Multiple pages of Sensors is supported in Custom. The 

Below is my custom ```conf.ini```

```ini
[Main]
style=Custom
sensors_per_line=3
pages=2
page_time=10

[PAGE1.Sensors]
sensor_0="RTSS;Framerate"
label_0="F"
unit_0=""

sensor_1="CLOCK"
label_1="⏰"
unit_1=""

sensor_2="BLANK"

sensor_3="GPU [#0]: NVIDIA GeForce RTX 3090;GPU Temperature"
label_3="⛏"
unit_3="°"

sensor_4="GPU [#0]: NVIDIA GeForce RTX 3090;GPU Core Load"
label_4=""
unit_4="%"

sensor_5="GPU [#0]: NVIDIA GeForce RTX 3090;GPU Power"
label_5=""
unit_5="W"

sensor_6="CPU [#0]: AMD Ryzen 9 7950X3D: Enhanced;CPU (Tctl/Tdie)"
label_6="💻"
unit_6="°"

sensor_7="CPU [#0]: AMD Ryzen 9 7950X3D;Total CPU Usage"
label_7=""
unit_7="%"

sensor_8="CPU [#0]: AMD Ryzen 9 7950X3D: Enhanced;CPU Package Power"
label_8=""
unit_8="W"

[PAGE2.Sensors]
sensor_0="System: ASUS ;Physical Memory Used"
label_0="RAM"
unit_0="g"
convert_0="MB/GB"

sensor_1="System: ASUS ;Physical Memory Available"
label_1=""
unit_1="g"
convert_1="MB/GB"

sensor_2="System: ASUS ;Physical Memory Load"
label_2=""
unit_2="%"

sensor_3="Network: Intel Ethernet Controller I225-V;Current UP rate"
label_3="NET ▲"
unit_3="k/s"

sensor_6="Network: Intel Ethernet Controller I225-V;Current DL rate"
label_6="NET ▼"
unit_6="k/s"
```

That produces these two pages:
```
F  00 ⏰05:56pm
⛏ 34° 01% 35W
💻 60° 06% 67W
```
```
RAM 15g 48g 23%
NET ▲ 01k/s  
NET ▼ 00k/s  
```
## Requirements
**HWiNFO**
https://www.hwinfo.com/

**SteelSeries GG**
https://steelseries.com/gg


## Steps for running:
- Make sure SteelSeries GG is running
- Make sure HWiNFO is running, Open the Sensors window
  - Click Start with Sensors checked
    
  ![hwinfo-sensors.png](/assets/hwinfo-sensors.png)
  - Enable "Shared Memory Support" in HWiNFO settings
    
  ![hwinfo-shared-memory.png](/assets/hwinfo-shared-memory.png)
- Run the `hwinfo-steelseries-oled.exe` file

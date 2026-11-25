# Solar (Victron)

Live PV power and solar yield (today, this month, this year, plus last
month and last year) read directly from a Victron system over the local
network. No VRM account and no internet access needed.

## Devices

The widget talks to whatever runs **Venus OS** in the system, not to the
solar devices themselves. Anything below that Venus OS can see is counted.

### Venus OS device (one per system)

| Device | Status |
|---|---|
| Cerbo GX | tested (Venus OS v3.85) |
| Cerbo-S GX, Ekrano GX, Venus GX, Octo GX | expected to work: same Venus OS |
| MultiPlus-II GX, EasySolar-II GX (built-in GX) | expected to work |
| Raspberry Pi running Venus OS | expected to work |
| Color Control GX (CCGX) | Modbus TCP and plain MQTT expected to work; MQTT over TLS needs a recent Venus OS |

### Solar sources

| Source | Live power | Totals |
|---|---|---|
| MPPT solar chargers (SmartSolar, BlueSolar, MPPT RS), VE.Direct or VE.Can, any number | yes | yes, history back-filled from the charger |
| PV inverters seen by Venus OS (Fronius, SMA, SolarEdge, ABB/Fimer, Victron energy meter measuring a PV inverter, ...), any number | yes | yes, counted from when the widget first sees them |
| PV on a Multi RS / Inverter RS (built-in MPPT) | yes | not counted yet |

Tested: SmartSolar MPPT 250/70 on a Cerbo GX. PV inverters are implemented
from the Venus OS register and topic lists, not tested on hardware.

## Connections

Enable the service in the Venus OS settings first; depending on the
version the page is called **Integrations** or **Services**.

| Connection | Enable on the device | Port | Notes |
|---|---|---|---|
| MQTT over TLS (default) | MQTT Access | 8883 | Encrypted. The device's certificate is self-signed, so it's accepted without checking. |
| MQTT, unencrypted | MQTT Access (plaintext allowed) | 1883 | Same data as TLS. |
| Modbus TCP | Modbus TCP server | 502 | No setup beyond enabling it; solar devices are found automatically. Only today and yesterday of history are available, so month/year start "Counting since" yesterday. |

Port 0 in the settings means the standard port of the chosen connection.

The widget only reads, and doesn't log in. If the network security profile
makes MQTT ask for a password, the widget shows "MQTT login refused"; use
Modbus TCP, or the "unsecured" profile, in that case. Tested with Venus OS
v3.85.

## How the totals work

Venus OS only keeps a lifetime counter and a short per-day history, so the
widget keeps its own month/year starting points on the Deck, under the
device's serial number:

- On first start they're back-filled from the history (31 days over MQTT,
  2 over Modbus). A period the history doesn't fully cover shows
  "Counting since <date>" until the next period starts.
- "Last month" / "Last year" appear once a whole period has been counted.
- Switching between MQTT and Modbus keeps the totals: both see the same
  serial number.
- Over Modbus the chargers' lifetime counter is 16-bit and rolls over at
  6553.5 kWh; the widget notices the rollover and keeps counting.

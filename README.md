# Ankabot

Command-line web fetcher with smart HTTP to Chrome fallback.

## Usage

Default (save PDF):

```bash
./ankabot https://yahoo.com
```

Custom path:

```bash
./ankabot --pdf out/yahoo.pdf https://yahoo.com
```

Disable PDF (just JSON/Screenshot):

```bash
./ankabot --no-pdf --screenshot yahoo.png https://yahoo.com
```

### Wait controls

Tune how long Chrome waits for the page to settle before printing. Defaults suit heavy portals:

```bash
./ankabot https://mail.ru \
  --wait-ready complete \
  --network-idle-ms 1200 \
  --max-wait-ms 15000
```

* `--max-wait-ms` (default `12000`): overall deadline for waits
* `--wait-ready` (default `complete`): document.readyState to await (`complete`|`interactive`|`none`)
* `--network-idle-ms` (default `1000`): how long network must stay idle
* `--wait-selector`: optional CSS selector to await

### Stateful profiles, cookies, and locale emulation

```bash
./ankabot https://yahoo.com \
  --profile yahoo-eu \
  --locale ru-RU --tz Europe/Moscow --geo "55.7558,37.6173" \
  --import-cookies cookies/yahoo.json \
  --export-cookies out/yahoo.cookies.json \
  --pdf out/yahoo.pdf
```

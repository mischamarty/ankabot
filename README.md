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

### Stateful profiles, cookies, and locale emulation

```bash
./ankabot https://yahoo.com \
  --profile yahoo-eu \
  --locale ru-RU --tz Europe/Moscow --geo "55.7558,37.6173" \
  --import-cookies cookies/yahoo.json \
  --export-cookies out/yahoo.cookies.json \
  --pdf out/yahoo.pdf
```

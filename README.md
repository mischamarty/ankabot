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

# Day-2 commands

The everyday loop once something is running.

[Back to the README](../README.md) · [Docs index](README.md)

Once something's running, the everyday loop:

```bash
zorvia vm list
zorvia vm get prod-db
zorvia vm status
zorvia vm status prod-db --watch
zorvia vm pause prod-db && zorvia vm resume prod-db
zorvia vm clone prod-db staging-db --start
zorvia snapshot snapshot-create prod-db --name before-upgrade
zorvia advisor health prod-db --detailed
zorvia change drift desired.yaml
zorvia change plan desired.yaml --vm prod-db
zorvia api tui --interactive
```

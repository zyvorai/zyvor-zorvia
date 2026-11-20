# Interactive TUI (overview)

The interactive Terminal User Interface is documented in **[INTERACTIVE_TUI.md](INTERACTIVE_TUI.md)** (canonical guide).

```bash
zorvia api tui
zorvia api tui --interactive
zorvia api tui --interactive --theme dark --namespace production
```

Widget implementation lives under `src/tui/widgets/` (dialogs, forms, menus, notifications, progress). Architecture and contributor notes: [DEVELOPMENT.md](../DEVELOPMENT.md).

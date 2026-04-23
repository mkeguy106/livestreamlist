# livestreamlist

A livestream monitoring UI with three switchable layouts — Linear/Vercel mono aesthetic, density 9.

The three titlebar dots at the top-left switch between layouts:

| Dot | Letter | Layout  | Shortcut |
| :-: | :----: | ------- | -------- |
| 1   | A      | Command | `1`      |
| 2   | B      | Columns | `2`      |
| 3   | C      | Focus   | `3`      |

- **Command** — sidebar rail + dense IRC chat, floating ⌘K palette.
- **Columns** — TweetDeck-style horizontal stream columns, each with its own chat.
- **Focus** — single featured stream with tab strip + 60/40 chat split.

The selected layout persists to `localStorage`. Keyboard shortcuts are ignored while typing in inputs.

## Stack

Vite + React 18. The UI is presently mocked data — API clients not yet wired.

## Develop

```bash
npm install
npm run dev       # http://localhost:5173
npm run build
npm run preview
```

## Structure

```
src/
├── App.jsx               # Titlebar + layout switcher
├── main.jsx              # React bootstrap
├── tokens.css            # Design tokens (zinc scale, platform colors, hairlines)
└── directions/
    ├── Command.jsx       # Layout A
    ├── Columns.jsx       # Layout B
    └── Focus.jsx         # Layout C
```

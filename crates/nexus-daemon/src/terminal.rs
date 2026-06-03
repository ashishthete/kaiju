//! In-browser interactive terminal: a WebSocket that mirrors an agent's tmux
//! pane (poll + repaint) and forwards keystrokes (raw `send-keys -H`).
//! Also serves the vendored xterm.js assets.

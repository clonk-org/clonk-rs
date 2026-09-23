# Enhanced in-game chat

Enable **Chat → Enhanced** in **Options → Advanced**. The setting is off by
default so the classic chat interface remains available. These preferences
change local presentation and input only; messages use the existing game
controls and recipient visibility rules.

Press **Enter** or **F2** to open chat. Recent messages appear in a compact panel
during play; opening chat expands the transcript and composer. **Shift+Enter**
opens allies chat and **Alt+Enter** opens speech above the selected crew.

- Click the recipient label to choose **Everyone**, **Allies**, **Say above crew**,
  or **Private → name**. **Ctrl+Tab** and **Ctrl+Shift+Tab** cycle recipients.
  Explicit commands such as `/team` and `/private` also update the label.
- Press **Enter** to send. **Esc** closes chat and keeps the draft. Each recipient
  has a separate draft. Select and delete the text to discard it.
- **Up/Down** browse messages sent to the selected recipient and restore the
  unfinished draft when returning to the end. Private message bodies stay in
  that recipient's history. **Tab/Shift+Tab** cycle matching player names or commands.
- Scroll over the transcript or press **Page Up/Page Down** to read history.
  Incoming messages preserve the reading position. Click **new messages ↓** or
  press **Ctrl+End** to return to the latest messages.
- Click **Conversations** or press **Ctrl+L** to include game logs. Recipient
  filtering happens before messages enter the panel.
- The bottom row changes **text size**, **background opacity**, **message duration**,
  and **timestamps**. Preferences are saved with the game's configuration.

Pasting never sends a message. Pasted line breaks become spaces for review in
the single-line composer; an oversized paste leaves the current draft intact
and shows an explanation. Failed submissions keep the composer and draft open.
Successful submission means the network worker accepted the message, not that
another player has acknowledged reading it.

The advanced configuration keys are:

```ini
[Chat]
Enhanced=1
TextSize=1
Opacity=85
Duration=12
```

`TextSize` is 0 (small), 1 (medium), or 2 (large). `Opacity` is 40–100 percent.
`Duration` is 3–60 seconds and affects the compact panel only. Expanded history
remains available until the round ends, within the bounded transcript buffer.

Scenario input prompts continue to use their classic input and callback behavior.

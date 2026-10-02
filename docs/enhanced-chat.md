# Enhanced in-game chat

Enhanced chat is the default for normal games, including existing installations
without a saved chat preference. These preferences change local presentation
and input only; messages use the existing game controls and recipient visibility
rules.

The LegacyClonk compatibility profile uses classic chat. In normal games,
**Options → Advanced → Chat → Enhanced** also allows an explicit choice of the
classic interface; that saved choice is respected on later launches.

Press **Enter** or **F2** to open chat. During play, recent messages appear directly
over the game with outlined text and no panel background. Up to six lines remain
visible; each message fades independently over its final two seconds. Long
messages show a two-line preview, with the full text available in history.
Opening chat expands the transcript and composer. **Shift+Enter**
opens allies chat and **Alt+Enter** opens speech above the selected crew.

Each message reads timestamp, channel, sender, then text. Timestamps are grey,
**[Allies]** is green and **[Private]** is pink for every sender, and the sender
appears in their player colour. With **White chat in game** on, the message
itself is white; otherwise it takes the sender's colour, as classic chat does.

- The recipient sits at the start of the composer, in its channel's colour.
  Click it to choose **Everyone**, **Allies**, **Say above crew**, or
  **Private → name**; the list opens above it and marks the current choice.
  **Ctrl+Tab** and **Ctrl+Shift+Tab** cycle recipients. Explicit commands such
  as `/team` and `/private` also update it. An empty composer names who will
  receive the message.
- Press **Enter** to send. **Esc** or **×** closes chat and keeps the draft. Each recipient
  has a separate draft. Select and delete the text to discard it.
- Untick **Show over game** to stop recent messages appearing over play while
  chat is closed. The choice holds until it is ticked again; it keeps the
  transcript and draft and does not mute other players.
- **Up/Down** browse messages sent to the selected recipient and restore the
  unfinished draft when returning to the end. Private message bodies stay in
  that recipient's history. **Tab/Shift+Tab** cycle matching player names or commands.
- Scroll over the transcript or press **Page Up/Page Down** to read history.
  Incoming messages preserve the reading position. Click **new messages ↓** or
  press **Ctrl+End** to return to the latest messages.
- The **Chat** tab shows conversation; **All** adds game messages. **Ctrl+L**
  switches between them. Recipient filtering happens before messages enter the
  panel.
- **Settings** opens Settings → General → Chat, which holds **text size**,
  **chat opacity**, **message duration**, **white chat** and **timestamps**.

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
The opacity setting applies only to the open history panel; previews remain
transparent. `Duration` is 3–60 seconds and affects previews only. Expanded history
remains available until the round ends, within the bounded transcript buffer.

Scenario input prompts continue to use their classic input and callback behavior.

The following captures use the game's sandbox render fixture:

![Recent messages over the game](images/chat-compact.png)

![Open chat history and composer](images/chat-expanded.png)

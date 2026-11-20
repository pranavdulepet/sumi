# Sumi

Sumi is a personal conversation app for Apple Silicon Macs. Tell it “My printmaking project is called Orchard.” Later, correct the name to “North Window.” Open a fresh sheet and ask it to draft an invitation. It can use the new name, show you the dated memory behind the answer, and let you change it again.

[Download Sumi for Mac](https://github.com/pranavdulepet/sumi/releases/tag/mac-1.0.7) · [Watch ten short walkthroughs](docs/walkthroughs.md) · [Build from source](docs/development.md)

## Start a conversation

On macOS 14 or later, unzip Sumi and move it to Applications. This release is ad-hoc signed. macOS may ask for approval to open it or access an existing notebook’s Keychain item after an upgrade.

Choose **Continue with ChatGPT** to connect an eligible plan, or **Think on this Mac instead** to use a local model. Draw your seal and write on the sheet. Return sends, Shift-Return adds a line, and **Sheets** (⌘O) reopens a conversation. You can import old chats after your first answer or from Settings.

Drop in a class schedule and ask “Which class can I take after work?” Follow up on the same sheet without attaching it again. Sumi reads PDFs, documents, images and spreadsheets on the Mac. Answers can include tables, charts, diagrams and checklists you can tick. Expand **Behind this reply** to inspect the model and tools.

For a weekend plan, tell Sumi your city and what you enjoy, then ask “What should I do this weekend?” Sumi works out the dates and previews the public query. **Search the web** checks current listings through Exa. When Sumi finds a match, the answer links to the venue. Settings can remember your permission for weekend listings.

## What Sumi learns

A red mark beside an answer opens the memory and its date. **Not true anymore** retires it. **What Sumi knows** (⌘K) collects current and earlier memories, people, projects and saved ways of working. **Correct this reply** lets you inspect a correction and try it on your next decision.

A standing instruction can become a named skill straight away. Similar requests on three distinct days, or positive feedback after a multi-step task, can lead to a skill during idle time. Skills are portable Markdown files. Sumi can revise them with you and archive learned skills after 60 days without use.

While the Mac is idle and plugged in, Sumi can compare a proposed memory with the existing one by predicting later replies. A worse aggregate score keeps the existing memory. Direct corrections take effect immediately. Settings shows the local measurements and how many observations they are based on.

## Models and privacy

Sumi routes each message across the models you connect. In ChatGPT mode, OpenAI receives the passages needed to respond. A connected local model can handle quick questions and private sheets. Local mode keeps model inference on the Mac. Without a local model, a sensitive question can offer a one-message preview for explicit sharing. Private sheets stay local. Attached file excerpts have their own sharing control.

Settings can connect an OpenRouter model for deeper answers or image generation with a request cap. Drawing an image asks for permission to send its prompt. Web search sends the displayed query. You can inspect and revoke each agent’s access to approved notebook entries.

Your notebook lives in `~/Sumi` by default. Conversations, dated memories and skills have readable Markdown copies. Anyone with access to those files can read them. The working database uses SQLCipher. Keys and optional provider credentials live in macOS Keychain. Settings can export a portable archive or move the notebook to the Trash. Keep the login Keychain with a complete notebook backup.

Sumi is MIT licensed. [Third-party notices](licenses/) · [Contributing](CONTRIBUTING.md) · [Requirements and verification](docs/requirements-review.md)

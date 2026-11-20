# Sumi

Sumi is a conversation app for Apple Silicon Macs. Ask for a weekend plan and it can combine what you’ve told it with current venue listings. The answer links to the venue. A small red mark opens the words that shaped the recommendation, their date, and **Not true anymore**.

[Download the latest Mac release](https://github.com/pranavdulepet/sumi/releases/latest) · [Watch ten walkthroughs](docs/walkthroughs.md) · [Build this branch](docs/development.md)

## Start a conversation

On macOS 14 or later, unzip Sumi and move it to Applications. The app uses an ad-hoc signature. macOS may ask for approval to open it or to access an existing notebook’s Keychain item after an upgrade.

Choose **Continue with ChatGPT** to connect an eligible plan, or **Think on this Mac instead** to use a local model. Draw a stroke for your seal, then write on the open sheet. Return sends. Shift-Return adds a line. **Sheets** (⌘O) reopens a conversation or starts a new one. Importing earlier conversations is optional.

Try telling Sumi “I live in Boston. On weekends I love hands-on printmaking workshops.” Open a fresh sheet and ask “What should I do this weekend?” Sumi resolves the weekend on your Mac, shows the public search query, and uses Exa’s keyless search endpoint when you click **Search the web**. If it needs a city, it asks. A dated venue excerpt accompanies the recommendation when available. Check **Automatically look up weekend listings** in Settings to skip the query preview for future weekend plans.

## Memory you can inspect

**What Sumi knows** (⌘K) shows dated notes, approved entries, earlier memories and saved methods. Sumi recognizes explicit statements such as a home city, weekend preference, or “My … is …” fact. A later statement about the same subject can supersede one you made earlier. A preference you retire disappears from current retrieval. The earlier statement and the answer that used it remain readable. **Correct this reply** lets you review Sumi’s interpretation and try the correction on the next decision.

An explicit recurring instruction can become a named skill immediately. Similar requests on three separate days, or positive feedback on a multi-step result, can lead to a skill during idle time. New skills have a portable `SKILL.md`. Earlier Markdown skills remain readable. An idle pass archives learned methods after 60 days without use.

While the Mac is idle and plugged in, Sumi can test a compact set of recurring memories against later conversation turns. It compares the local model’s predictions with and without the proposed change. A lower aggregate score keeps the previous core. Direct corrections take effect immediately. A new notebook needs enough distinct days and held-out follow-ups before a core is promoted.

## Models, search and files

Sumi can select a calculator, a civil-date calculation, earlier passages, a saved skill or web search. The conversation renders lists, tables and bar charts. **Behind this reply** shows the model and tools. **Settings** (⌘,) manages intelligence, agent access, readings and chat imports.

In ChatGPT mode, OpenAI receives the passages needed for the request. Sumi can use a local model for selection and quick replies, a fast eligible plan model when available, and the main plan model for deeper work. A private sheet requires local inference. Local mode keeps inference on the Mac. An approved web search sends its public query to Exa. An explicitly offline sheet disables search. Other provider connections support specific background jobs.

Your notebook lives in `~/Sumi` by default. Read conversations in `desk/conversations/`, dated notes in `desk/memories.md`, the promoted core in `desk/core.md`, and skills in `scroll/skills/`. These Markdown files are readable by anyone with access to the folder. The encrypted database in `.sumi/` holds the working state. macOS Keychain holds its key and optional provider credentials. Keep your login Keychain with your notebook backup.

Sumi is MIT licensed. Bundled art and runtime notices are under [licenses](licenses/). See [Contributing](CONTRIBUTING.md) for feedback and code changes.

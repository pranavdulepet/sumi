# Sumi

Sumi is a personal notebook for conversations and decisions on Apple Silicon Macs. Ask a question, correct an answer in your own words, and choose which parts of that correction belong in the notebook. On the next related decision, you can open Sumi’s guess and see the sentence it read.

[Download the latest Mac release](https://github.com/pranavdulepet/sumi/releases/latest) · [Watch ten walkthroughs](docs/walkthroughs.md) · [Build the current branch](docs/development.md)

## Open your notebook

On macOS 14 or later, unzip the download and move Sumi to Applications. The downloadable app uses an ad-hoc signature. macOS may require approval to open it and to access an existing notebook’s Keychain item after upgrading.

Draw your mark. Choose **Continue with ChatGPT** to use your eligible plan, or **Think on this Mac instead** to use an installed model or download a verified one. Once connected, Sumi opens a sheet where you can ask your first question.

## At the Desk

Press Return to send and Shift-Return for a new line. Ask a follow-up on the same sheet, or open **Sheets** (⌘O) to start again. Sumi can select a calculator, look up civil dates on your Mac, and retrieve dated passages from earlier conversations. For example, tell Sumi your class budget on one sheet, then ask on another how much you can spend per class. **Behind this reply** shows the tools, model and method used.

Tell Sumi how you like recurring work done. It can name a skill and save your instruction automatically. A later instruction can update that skill. When a local model is available, Sumi can also consolidate repeated requests while your Mac is idle and plugged in. In the Scroll, you can inspect, edit or remove a method. To check what changed, ask Sumi to show your earlier instructions in order.

**Correct this reply** opens the guided correction flow. You can stamp an editable preference and try it on a sealed decision. The regular conversation works before you do any of that.

Use **Places** or ⌘1–4 for the Desk, Scroll, Mirror and Seal. The Mirror counts scored guesses and lets you inspect each verdict. The Seal manages access for agents you connect. Open **Settings** with ⌘, to change your thinking setup, manage sources or bring in a chat export. Importing history is optional.

## Models and files

In ChatGPT mode, the passages needed for an answer go to OpenAI. Sumi uses an available local model for tool selection and simple replies, and the connected plan for more involved requests. With no local model available, the plan can handle selection as well as the response. Select **Keep this sheet local** before entering private material. A private sheet requires local inference for its replies. In local mode, all conversation turns use the local model. Other provider connections in Settings support specific background jobs.

Your notebook lives in `~/Sumi` by default. Drafts live in `desk/`, readable conversations in `desk/conversations/`, and editable skills in `scroll/skills/`. These Markdown files are readable by anyone with access to that folder. The encrypted database in `.sumi/` holds observations, conversation state and scores. macOS Keychain holds the database key and optional provider credentials. Keep your login Keychain with your notebook backup.

The source is MIT licensed. Bundled assets and runtime notices are under [licenses](licenses/). See [Contributing](CONTRIBUTING.md) for feedback and code changes.

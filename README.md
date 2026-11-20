# Sumi

Sumi is a personal notebook for conversations and decisions on Apple Silicon Macs. Ask a question, correct an answer in your own words, and choose which parts of that correction belong in the notebook. On the next related decision, you can open Sumi’s guess and see the sentence it read.

[Download the latest Mac release](https://github.com/pranavdulepet/sumi/releases/latest) · [Watch ten walkthroughs](docs/walkthroughs.md) · [Build the current branch](docs/development.md)

## Open your notebook

On macOS 14 or later, unzip the download and move Sumi to Applications. The downloadable app uses an ad-hoc signature. macOS may require approval to open it and to access an existing notebook’s Keychain item after upgrading.

Draw your mark. Choose **Continue with ChatGPT** to use your eligible plan, or **Think on this Mac instead** to use an installed model or download a verified one. Start with a real correction. Read the sentence Sumi learned, edit it, and keep it. Write down a related decision, record your choice, and open the seal to compare.

## At the Desk

Write on a sheet and choose **Ask Sumi** or press ⌘Return. Continue on the same sheet for follow-ups. **Behind this reply** shows the model, any preferences supplied to it, and any skill it used. **Correct this reply** starts a correction with the exchange in view. **Sheets** (⌘O) opens your earlier conversations.

When you repeat a way of working, Sumi can offer to keep it as a skill. Review its steps, give it a name, and stamp it. You can also choose **Make this a skill** on any reply. A future request that matches the skill’s name can use it automatically, or you can pick it in the margin. Edit or remove it in the Scroll.

Use **Places** or ⌘1–4 for the Desk, Scroll, Mirror and Seal. The Mirror counts scored guesses and lets you inspect each verdict. The Seal manages access for agents you connect. Open **Settings** with ⌘, to change your thinking setup, manage sources or bring in a chat export. Importing history is optional.

## Models and files

In ChatGPT mode, the passages needed for an answer go to OpenAI. Sumi tries an available local model for short tasks such as rewriting or translation. Select **Keep this sheet local** before entering private material. A private sheet requires local inference for its replies. In local mode, all conversation turns use the local model. Other provider connections in Settings support specific background jobs.

Your notebook lives in `~/Sumi` by default. Drafts live in `desk/`, readable conversations in `desk/conversations/`, and editable skills in `scroll/skills/`. These Markdown files are readable by anyone with access to that folder. The encrypted database in `.sumi/` holds observations, conversation state and scores. macOS Keychain holds the database key and optional provider credentials. Keep your login Keychain with your notebook backup.

The source is MIT licensed. Bundled assets and runtime notices are under [licenses](licenses/). See [Contributing](CONTRIBUTING.md) for feedback and code changes.

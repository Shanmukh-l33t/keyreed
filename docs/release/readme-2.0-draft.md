# README changes for 2.0 (draft)

Draft by lane B. Each section below says where it goes in `README.md`. Every claim is marked **[confirm before publishing]**: check it against the release build, delete what did not ship, and fill each [bracket] with a measured figure or remove the sentence. Claims come from plan sections A, B, C1, C2 and C3 only. Version numbers, file names and sizes in Quick start are not drafted here; take them from the release.

---

## 1. Opening paragraph (replaces the second paragraph under the title)

**KeyReed is an open-source, local-first voice dictation and writing app for Windows.** Hold a shortcut, speak, release, and KeyReed types clean text into whatever application you were using. Select text and press a second shortcut, and it fixes the typos. Click the pill, and a small window rewrites, shortens or drafts text for you. It all runs on your own PC. **[confirm before publishing]**

## 2. Why KeyReed (add rows to the table)

| | |
| --- | --- |
| **Speak it, fix it, write it** | Dictation, a selection fixer and a writing window in one app, on your PC. **[confirm before publishing]** |
| **Learns your words** | Words you correct go into your dictionary, with a readable list of what it learned and one switch to turn it off. **[confirm before publishing]** |
| **Light on low-end PCs** | A performance profile keeps memory and threads small on PCs with four or fewer logical processors or 8 GB of RAM or less. **[confirm before publishing]** |

## 3. Features (add after "Personal vocabulary", before "Output choice")

- **Quick Fix.** Select text in almost any application, press **Ctrl+Win+G** [confirm the default], and KeyReed corrects typos, missing apostrophes, doubled words and common slips. A review panel shows every changed word; Enter replaces the selection, Esc leaves it alone. Protected text (links, e-mail addresses, paths, numbers, names, code and shell commands) is never changed. Undo stays on the pill for a few seconds. Replacing without review is a setting, and only for the kinds of fix that passed KeyReed's own accuracy test [list them]. English only. **[confirm before publishing]**
- **Deeper pass.** A second look by a language model, shown for review and never applied by itself. [Say where it runs.] **[confirm before publishing]**
- **The Composer.** Click the pill, or press **Ctrl+Win+K** [confirm the default]: a small window beside your text with Fix, Improve, Shorter, Change tone, Summarise and your own instruction. Follow-ups work on the latest result, the answer streams and Esc cancels, and you can Insert, Replace, Copy or Regenerate. A chip shows the selection that will be used, and a label shows where the work runs ("On this PC, model name" or "Cloud, host, model"). Dictation works inside it. **[confirm before publishing]**
- **Writing models.** The bundled small model is the default. Optional downloads: Qwen3.5 2B (about 1.3 GB) and 4B (about 2.7 GB), both Apache-2.0, verified by size and SHA-256, one loaded at a time; KeyReed suggests one for your PC's memory, processor count and GPU. **[confirm before publishing]**
- **My style.** Learns words from your corrections (a correction seen twice goes into the dictionary with an Undo; seen once, it waits in a list) and counts writing habits from text you hand to Fix or Improve and from samples you paste in (spelling dialect, contractions, sentence length, serial comma, dashes, full stops on short messages, emoji). The Style Card lists each habit with its evidence and lets you Keep, Edit or Remove it. Quick Fix leaves habits you hold alone; the Composer gets a short style note. Pause, Reset, Export and Delete everything are in Settings. Stored in `style.json` next to your history file. **[confirm before publishing]**
- **Press-your-shortcut boxes** in Settings, with a clear message when a shortcut clashes with a common editing shortcut or with another KeyReed shortcut. **[confirm before publishing]**
- **Performance profile** (Auto, Standard, Light). **[confirm before publishing]**

## 4. Usage table (add rows)

| Action | Default |
| --- | --- |
| Quick Fix the selection | **Ctrl+Win+G** [confirm] |
| Open the Composer | **Ctrl+Win+K** [confirm], or click the pill |
| Apply a spoken instruction to the selection | [confirm whether it shipped, and its shortcut] |

Add one sentence under the table: "Clicking the pill used to start dictation. It now opens the Composer; **When I click the pill** in Settings can restore the old click or turn it off. Dictation by shortcut is unchanged." **[confirm before publishing]**

## 5. Privacy and architecture (add this bullet first, and the clipboard bullet after the existing clipboard one)

- What you say, write and correct stays on this PC. KeyReed never logs keystrokes and never captures the screen. To learn your words it re-checks the text box it just typed into, for up to a minute, compares in memory and keeps only the corrected words. One switch turns that off. **[confirm before publishing]**
- Quick Fix reads your selection through the Windows clipboard, so that copy can appear in clipboard history or a clipboard manager. KeyReed saves your clipboard first and puts it back afterwards, and stops with a message if it cannot. **[confirm before publishing]**
- Learning never reads password fields, terminals, password managers, private-browsing windows, administrator windows, apps on your exclusion list, or browser-based apps (Chrome, Edge, Slack and similar are not read in 2.0). **[confirm before publishing]**
- The default profile still makes no network call for audio or text. The Composer and the deeper pass run on this PC unless you switch on the separate cloud switch for text jobs, which is off by default. **[confirm before publishing]**

The mermaid diagram needs a second branch for text jobs (`Selection → Quick Fix / Composer → your PC`, with the same dotted cloud arrow). Draw it only if the feature shipped. **[confirm before publishing]**

## 6. System requirements (adjust)

- Disk: add "the grammar model, [112 MB], downloaded on first use of Quick Fix, and optionally a 1.3 GB or 2.7 GB writing model". **[confirm before publishing]**
- RAM and speed: [only the figures measured on the release build; the 0.5.1 text says an 8 GB PC was not benchmarked, so keep that sentence unless it was].

## 7. Known limits (add)

- Quick Fix catches common slips, not every error, and is built to leave correct text alone. Its accuracy figures come from KeyReed's own test set [publish the numbers and the set]. **[confirm before publishing]**
- It does not work in administrator windows, password fields or terminals, and it does not keep the formatting inside a selection with links, tables or lists (it opens the review panel instead). **[confirm before publishing]**
- Learning from your edits works in native Windows applications only; browser-based applications are not read. [List the applications actually checked.] **[confirm before publishing]**
- The bundled small writing model is weaker than the optional 2B and 4B models. **[confirm before publishing]**
- English only for Quick Fix. **[confirm before publishing]**

## 8. Roadmap table (add rows)

| Status | Item |
| --- | --- |
| Done | Quick Fix, the Composer, optional writing models, My style (2.0) **[confirm before publishing]** |
| Open | Reading browser-based windows for learning, only if it proves cheap enough |
| Open | A grammar model KeyReed trains itself, for stronger offline correction |
| Open | Fix last dictation: correct a dictated word in a window and have KeyReed learn it with certainty |

The last two Open rows are in the plan as later work, not commitments: keep or delete by the main builder's judgement. **[confirm before publishing]**

## 9. Status line

Replace "release **0.5.1**" with the release version once it is published. Do not claim accuracy parity with Grammarly, multilingual correction, "nothing ever leaves the device" when a cloud key is on, or "works in every app"; the plan lists these as claims to avoid.

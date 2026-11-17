#!/usr/bin/env python3
"""Builds bench/quickfix_gate.jsonl: the typed-text gate set for Quick Fix (plan P0.1).

Every sentence is written for this set; nothing is copied from a licensed corpus.
Each case: id, category, context, input, reference, must_keep, must_not_contain.
  - Clean cases (clean_everyday, clean_technical) have input == reference: the engine must change nothing.
  - must_keep always holds the protected-looking tokens (URLs, paths, numbers, identifiers, flags) found in both
    input and reference, plus any tokens named explicitly (names, commands).
  - must_not_contain lists wrong forms that must not appear in the output (the seeded misspelling, "revert back").
Run: python bench/build_quickfix_gate.py   (rewrites bench/quickfix_gate.jsonl, checks the set's invariants)
"""
import json
import re
from collections import Counter
from pathlib import Path

OUT = Path(__file__).with_name("quickfix_gate.jsonl")
CATEGORIES = ["spelling", "agreement", "confusable", "article", "punctuation", "doubled_word", "capitalisation",
              "indian_english", "clean_everyday", "clean_technical", "protected"]
CONTEXTS = ["email", "chat", "document", "notes", "code", "general"]

CASES = []  # (category, context, input, reference, explicit keep, must_not_contain)


def e(cat, ctx, inp, ref, keep=(), bad=()):
    """A case with at least one seeded error (input != reference)."""
    CASES.append((cat, ctx, inp, ref, tuple(keep), tuple(bad)))


def ok(cat, ctx, text, keep=()):
    """A correct text the engine must leave exactly as it is."""
    CASES.append((cat, ctx, text, text, tuple(keep), ()))


# ---------------------------------------------------------------- clean_everyday (input == reference)
ok("clean_everyday", "email", "Hi Priya, thanks for sending the invoice over so quickly.")
ok("clean_everyday", "email", "I will be out of the office from Monday and back on the 14th.")
ok("clean_everyday", "email", "Could you please confirm whether the meeting is still at 3 pm tomorrow?")
ok("clean_everyday", "email", "Thank you for your patience while we looked into this.")
ok("clean_everyday", "email", "Attached is the revised schedule, and I have highlighted the changes in yellow.")
ok("clean_everyday", "email", "We would like to move the review to Thursday if that works for everyone.")
ok("clean_everyday", "email", "Please let me know if you need anything else before the end of the week.")
ok("clean_everyday", "email", "I am sorry for the late reply; the last few days have been very busy.")
ok("clean_everyday", "email", "Best regards, Ananya")
ok("clean_everyday", "email", "Neither of the two options is ideal, but the second one costs less.")
ok("clean_everyday", "email", "The committee have decided to postpone the vote until next month.")
ok("clean_everyday", "email", "Just a quick reminder that the survey closes on Friday.")
ok("clean_everyday", "email", "Fewer than ten people replied, so we will send a second reminder.")
ok("clean_everyday", "email", "She said that she had had enough of the delays, and I do not blame her.")
ok("clean_everyday", "email", "I would be grateful if you could look at the draft before lunch.")
ok("clean_everyday", "email", "Our offices will be closed on Monday for the public holiday.")
ok("clean_everyday", "chat", "ok sounds good, see you at 6")
ok("clean_everyday", "chat", "lol that's hilarious, send me the pic")
ok("clean_everyday", "chat", "Running about ten minutes late, sorry!")
ok("clean_everyday", "chat", "Did you get my message about dinner?")
ok("clean_everyday", "chat", "I'm on my way, can you grab me a coffee?")
ok("clean_everyday", "chat", "Thanks a ton, that really helped.")
ok("clean_everyday", "chat", "who's bringing the snacks on Saturday?")
ok("clean_everyday", "chat", "Haven't heard back from them yet, will ping again tomorrow.")
ok("clean_everyday", "chat", "I'd rather you didn't tell her until after the party.")
ok("clean_everyday", "chat", "That's not what I meant, but fair enough.")
ok("clean_everyday", "chat", "Can we talk after lunch? It's kind of urgent.")
ok("clean_everyday", "chat", "Everyone is here except Karthik, so we can start.")
ok("clean_everyday", "chat", "it's fine, no worries")
ok("clean_everyday", "chat", "yeah I think so, but let me double check with Meera first")
ok("clean_everyday", "chat", "Good morning! Hope the weekend was nice.")
ok("clean_everyday", "chat", "None of them were late, which is a first.")
ok("clean_everyday", "document", "The meeting will begin at nine in the morning and finish before noon.")
ok("clean_everyday", "document", "Each employee is entitled to twenty days of paid leave per year.")
ok("clean_everyday", "document", "The report summarises the main findings and lists the open questions.")
ok("clean_everyday", "document", "Section 3 describes how the budget was agreed and who signed it off.")
ok("clean_everyday", "document", "Applicants must submit two references and a short statement of purpose.")
ok("clean_everyday", "document", "In the first quarter, sales rose by six percent compared with last year.")
ok("clean_everyday", "document", "The staff are meeting the new director on Wednesday.")
ok("clean_everyday", "document", "A full refund is available within thirty days of purchase.")
ok("clean_everyday", "notes", "buy milk, eggs and bread on the way home")
ok("clean_everyday", "notes", "Call the dentist on Tuesday to move the appointment.")
ok("clean_everyday", "notes", "Ideas for the offsite: a hike, a cooking class, or a quiet day at the lake.")
ok("clean_everyday", "notes", "Reminder to renew the car insurance before the 20th.")
ok("clean_everyday", "notes", "Talk to Rohan about the new hire, then update the plan.")
ok("clean_everyday", "notes", "Meeting notes: we agreed to ship on Friday and review the numbers on Monday.")
ok("clean_everyday", "notes", "Todo: pick up the parcel, water the plants, and reply to Divya.")
ok("clean_everyday", "notes", "Book a table for six at 7:30 on Saturday.")
ok("clean_everyday", "notes", "Things I liked about the talk: clear examples, honest limits, and no jargon.")
ok("clean_everyday", "notes", "The recipe needs two cups of flour, a pinch of salt, and one egg.")
ok("clean_everyday", "notes", "Follow up with the landlord about the broken heater.")
ok("clean_everyday", "notes", "Whom should I thank for the flowers? Ask Sunita.")
ok("clean_everyday", "general", "The weather has been unusually warm for this time of year.")
ok("clean_everyday", "general", "I have been learning to play the guitar for about six months.")
ok("clean_everyday", "general", "My brother lives in Pune, and my sister lives in Hyderabad.")
ok("clean_everyday", "general", "There are three kinds of tea in the cupboard.")
ok("clean_everyday", "general", "If I were you, I would take the earlier train.")
ok("clean_everyday", "general", "The children were playing in the garden when it started to rain.")
ok("clean_everyday", "general", "He doesn't like spicy food, but she loves it.")
ok("clean_everyday", "general", "Their new flat is much closer to the station than ours.")
ok("clean_everyday", "general", "It's been a long day, and I think the dog needs a walk.")
ok("clean_everyday", "general", "We could not find the keys anywhere, so we called a locksmith.")
ok("clean_everyday", "general", "Do you know where the nearest pharmacy is?")
ok("clean_everyday", "general", "The film was longer than I expected, but the ending was worth it.")

# ---------------------------------------------------------------- clean_technical (input == reference)
ok("clean_technical", "notes", "Modifier state is a set of flags that the hook reads on every key event.")
ok("clean_technical", "notes", "Process CPU time stays flat while the model is idle.")
ok("clean_technical", "notes", "The cache key is the SHA-256 of the file contents, not the file name.")
ok("clean_technical", "notes", "Each worker owns a bounded channel, so a slow consumer cannot grow memory.")
ok("clean_technical", "notes", "The parser returns None when the header is shorter than 12 bytes.")
ok("clean_technical", "notes", "Latency is measured from the key press to the first token, in milliseconds.")
ok("clean_technical", "notes", "The retry loop backs off exponentially and gives up after five attempts.")
ok("clean_technical", "notes", "Memory use peaks during model load and drops back once the weights are mapped.")
ok("clean_technical", "notes", "The benchmark runs three warm-up iterations before timing starts.")
ok("clean_technical", "notes", "Disk writes are atomic: the file is written to a temporary name and then renamed.")
ok("clean_technical", "notes", "A mutex guards the session handle, so only one inference runs at a time.")
ok("clean_technical", "notes", "The tokenizer splits on whitespace first and then applies the merge table.")
ok("clean_technical", "notes", "Threads are pinned to physical cores on machines with more than four cores.")
ok("clean_technical", "notes", "Quantisation reduces the model from 446 MB to 112 MB with the same accuracy.")
ok("clean_technical", "notes", "The idle timeout unloads the model after ten minutes without a request.")
ok("clean_technical", "notes", "Edge cases: empty input, input with only whitespace, and input over 8,000 characters.")
ok("clean_technical", "document", "The configuration file is read once at startup and cached in memory.")
ok("clean_technical", "document", "All requests must include an API key in the Authorization header.")
ok("clean_technical", "document", "The service listens on port 8080 by default and can be changed with the PORT variable.")
ok("clean_technical", "document", "Logs contain timings and process names but never the text that was typed.")
ok("clean_technical", "document", "Release builds are stripped and statically linked against the C runtime.")
ok("clean_technical", "document", "The installer verifies the SHA-256 of every downloaded model before it is loaded.")
ok("clean_technical", "document", "Clipboard contents are restored once the paste has completed.")
ok("clean_technical", "document", "A request that exceeds the size limit is rejected with status code 413.")
ok("clean_technical", "document", "The database schema is versioned, and migrations run in order at startup.")
ok("clean_technical", "document", "Both the CPU and the GPU paths produce identical output for the same input.")
ok("clean_technical", "document", "The default language is English; other languages are not supported yet.")
ok("clean_technical", "document", "This setting takes effect after the application restarts.")
ok("clean_technical", "code", "This function returns the number of bytes written, or an error if the write fails.")
ok("clean_technical", "code", "TODO: handle the case where the buffer is empty before calling parse_header.")
ok("clean_technical", "code", "The constructor takes ownership of the buffer and frees it in drop.")
ok("clean_technical", "code", "Use a BTreeMap here so that the iteration order is stable between runs.")
ok("clean_technical", "code", "Returns true if the path exists and is readable by the current user.")
ok("clean_technical", "code", "The lock is held for the whole duration of the call, so keep the closure short.")
ok("clean_technical", "code", "Panics if the index is out of range; use get for a checked lookup.")
ok("clean_technical", "code", "These tests need a display and are skipped on CI.")
ok("clean_technical", "code", "Fix the off-by-one error in the ring buffer when the head wraps around.")
ok("clean_technical", "code", "The test creates its own window and restores the clipboard afterwards.")
ok("clean_technical", "code", "Refactor the loader so it no longer reads the whole file into memory.")
ok("clean_technical", "code", "Bump the minimum supported Rust version to 1.75.")
ok("clean_technical", "chat", "the build is green again after I pinned the compiler version")
ok("clean_technical", "chat", "Can you check why the CI job times out after 10 minutes?")
ok("clean_technical", "chat", "I rebased onto main and force-pushed my own branch, nobody else's.")
ok("clean_technical", "chat", "The staging server is down again, so the deploy has to wait.")
ok("clean_technical", "chat", "Looks like the cache was stale, so I cleared it and re-ran the tests.")
ok("clean_technical", "chat", "We should add a timeout here, otherwise one slow request blocks the queue.")
ok("clean_technical", "chat", "All tests pass locally, but two fail on the Linux runner.")
ok("clean_technical", "chat", "Memory usage doubled after the last merge, so I'm bisecting now.")
ok("clean_technical", "chat", "The logs show a 502 right before the restart, not after it.")
ok("clean_technical", "chat", "Which branch has the latest fix for the installer?")
ok("clean_technical", "chat", "Nope, the flag only affects release builds.")
ok("clean_technical", "chat", "The issue is reproducible on Windows 11 but not on Windows 10.")
ok("clean_technical", "email", "The upgrade completed at 02:15 UTC and no data was lost.")
ok("clean_technical", "email", "Please find the incident report attached; the root cause was an expired certificate.")
ok("clean_technical", "email", "The new release reduces start-up time by roughly forty percent on older laptops.")
ok("clean_technical", "email", "Our backup job runs nightly and keeps the last thirty snapshots.")
ok("clean_technical", "email", "We recommend rotating the access keys every ninety days.")
ok("clean_technical", "email", "The vendor confirmed that the patch fixes the memory leak in version 4.2.1.")
ok("clean_technical", "email", "Downtime was limited to eleven minutes while the database failed over.")
ok("clean_technical", "email", "Could you share the output of the diagnostics tool when you have a moment?")
ok("clean_technical", "email", "The firewall rule allows inbound traffic only from the office network.")
ok("clean_technical", "email", "Two of the servers are still on the old kernel and need a reboot.")
ok("clean_technical", "general", "A byte is eight bits, and a kilobyte is 1,024 bytes in this context.")
ok("clean_technical", "general", "The laptop has 16 GB of RAM and a 512 GB solid-state drive.")
ok("clean_technical", "general", "Wi-Fi speeds drop quickly as you move away from the router.")
ok("clean_technical", "general", "The battery lasts about nine hours with the screen at half brightness.")

# ---------------------------------------------------------------- protected (URLs, paths, identifiers, numbers, names, commands)
# URLs and addresses
e("protected", "email", "Plese open https://docs.keyreed.app/install/windows and follow the steps.",
  "Please open https://docs.keyreed.app/install/windows and follow the steps.")
ok("protected", "chat", "The dashboard is at http://localhost:3000/admin?tab=users if you want to look.")
e("protected", "email", "Send the report to ananya.rao@example.com befor noon.",
  "Send the report to ananya.rao@example.com before noon.")
ok("protected", "document", "Source code is published at https://github.com/example-org/widget-kit under the MIT licence.")
e("protected", "chat", "dont forget to check www.status-page.example.org/incidents before you restart",
  "don't forget to check www.status-page.example.org/incidents before you restart")
ok("protected", "email", "Questions can go to support@keyreed.example or to the shared inbox.")
e("protected", "notes", "the api docs is at https://api.example.com/v2/docs#authentication",
  "the api docs are at https://api.example.com/v2/docs#authentication")
ok("protected", "notes", "Bookmark https://example.com/a/b/c.html?utm=1&ref=abc and read it tonight.")
# Paths
ok("protected", "chat", r"The installer is at C:\Users\Meera\Downloads\KeyReedSetup.exe on her laptop.")
e("protected", "notes", r"Config lives in %APPDATA%\KeyReed\config.json but the old one is stil in ~/.keyreed/settings.toml",
  r"Config lives in %APPDATA%\KeyReed\config.json but the old one is still in ~/.keyreed/settings.toml")
ok("protected", "code", "Edit /etc/nginx/conf.d/default.conf and reload the service.")
e("protected", "code", "the logs are written to /var/log/keyreed/app.log, rotated evry night",
  "the logs are written to /var/log/keyreed/app.log, rotated every night")
ok("protected", "notes", r"Copy D:\Projects\Reports\Q3_summary_final.xlsx to the shared drive.")
e("protected", "document", "Save the file as notes_2026-11-17.md in the docs/design folder, then comit it.",
  "Save the file as notes_2026-11-17.md in the docs/design folder, then commit it.")
# Identifiers and code words
e("protected", "code", "The function parse_header returns None when the buffer are empty.",
  "The function parse_header returns None when the buffer is empty.")
ok("protected", "code", "Rename getUserName to fetchUserName in the AuthService class.")
e("protected", "code", "set MAX_RETRY_COUNT to 5 becuase the default is too low",
  "set MAX_RETRY_COUNT to 5 because the default is too low")
ok("protected", "notes", "The crate vi_core exports AppConfig, HotkeyEvent and UiAction.")
e("protected", "code", "call serde_json::from_str on the raw string, then check the vesion field",
  "call serde_json::from_str on the raw string, then check the version field")
ok("protected", "chat", "Did the build fail on ci_linux_x64 or on the windows runner?")
e("protected", "notes", "the env var KEYREED_RUNTIME_ROOT must point at the folder with the modles",
  "the env var KEYREED_RUNTIME_ROOT must point at the folder with the models")
ok("protected", "code", "Check the isEnabled flag before calling renderOverlay().")
# Numbers, versions, times, money
ok("protected", "email", "The total came to Rs. 48,250.75 including GST at 18%.")
e("protected", "email", "Please pay the invoice INV-2026-0457 of $1,240.50 by 30 November, thankyou.",
  "Please pay the invoice INV-2026-0457 of $1,240.50 by 30 November, thank you.")
ok("protected", "notes", "Build 0.5.1 is 76.4 MB and the 2.0 beta is about 112 MB.")
e("protected", "chat", "the call is at 15:30 IST on 2026-11-20, dont be late",
  "the call is at 15:30 IST on 2026-11-20, don't be late")
ok("protected", "document", "Call +91 98765 43210 between 9:00 and 17:30 on weekdays.")
e("protected", "notes", "port 8443 and 9090 is open on the firewall, 22 is closed",
  "port 8443 and 9090 are open on the firewall, 22 is closed")
# Names
e("protected", "email", "Please ask Shanmukh Dheekonda abuot the release schedule.",
  "Please ask Shanmukh Dheekonda about the release schedule.", keep=["Shanmukh Dheekonda"])
ok("protected", "chat", "Tell Will that Rose called about the Bill.", keep=["Will", "Rose", "Bill"])
e("protected", "email", "Priya met Dr. Venkatesh at the Sathya Sai clinic and they discussed teh results.",
  "Priya met Dr. Venkatesh at the Sathya Sai clinic and they discussed the results.", keep=["Venkatesh", "Sathya Sai"])
ok("protected", "document", "The proposal was reviewed by Kavita Nair and approved by Mr. Iyer.", keep=["Kavita Nair", "Iyer"])
e("protected", "chat", "Meera and Karthik was at the standup but Divya wasnt",
  "Meera and Karthik were at the standup but Divya wasn't", keep=["Meera", "Karthik", "Divya"])
ok("protected", "notes", "Spoke to Hope from Marks and Spencer about the Ghibli tickets.", keep=["Hope", "Marks", "Spencer", "Ghibli"])
e("protected", "email", "Thanks to Sunita Rao, Arjun and Mohan for there help with the launch.",
  "Thanks to Sunita Rao, Arjun and Mohan for their help with the launch.", keep=["Sunita Rao", "Arjun", "Mohan"])
# Shell commands
e("protected", "code", "Run cargo build --release befor you push.",
  "Run cargo build --release before you push.", keep=["cargo build --release"])
ok("protected", "notes", "Run `git log --oneline -20` to see the recent commits.", keep=["git log --oneline -20"])
ok("protected", "chat", "Then type sudo apt install ffmpeg and press enter.", keep=["sudo apt install ffmpeg"])
e("protected", "code", "use git rebase -i HEAD~3 to squash the last three comits.",
  "use git rebase -i HEAD~3 to squash the last three commits.", keep=["git rebase -i HEAD~3"])
e("protected", "notes", "docker run -p 8080:80 nginx starts the container, then open teh browser",
  "docker run -p 8080:80 nginx starts the container, then open the browser", keep=["docker run -p 8080:80 nginx"])
ok("protected", "code", "curl -s https://api.example.com/v1/status | jq .ok", keep=["curl -s https://api.example.com/v1/status | jq .ok"])
e("protected", "chat", "run pip install -r requirements.txt then restart teh server",
  "run pip install -r requirements.txt then restart the server", keep=["pip install -r requirements.txt"])
ok("protected", "notes", "ls -la ~/projects && cd keyreed && cargo test --workspace", keep=["ls -la ~/projects && cd keyreed && cargo test --workspace"])

# ---------------------------------------------------------------- spelling
e("spelling", "email", "I will recieve the parcel tommorow morning.", "I will receive the parcel tomorrow morning.")
e("spelling", "email", "We definately need to seperate the two accounts.", "We definitely need to separate the two accounts.")
e("spelling", "email", "The problem occured again untill we restarted the server.", "The problem occurred again until we restarted the server.")
e("spelling", "email", "Please let me know wich option you prefer.", "Please let me know which option you prefer.")
e("spelling", "email", "I beleive the goverment office opens at nine.", "I believe the government office opens at nine.")
e("spelling", "email", "Our new adress is on the second floor of the building.", "Our new address is on the second floor of the building.")
e("spelling", "email", "It is neccessary to submit the form before the begining of the term.",
  "It is necessary to submit the form before the beginning of the term.")
e("spelling", "email", "Thankyou for the quick turnaround, it was very helpfull.", "Thank you for the quick turnaround, it was very helpful.")
e("spelling", "email", "The commitee will meet to discuss the arguement on Thursday.", "The committee will meet to discuss the argument on Thursday.")
e("spelling", "email", "We are sorry for the inconvienience this caused.", "We are sorry for the inconvenience this caused.")
e("spelling", "chat", "i dont think thats right, im checking again", "I don't think that's right, I'm checking again")
e("spelling", "chat", "I cant make it tonight, ive got a deadline.", "I can't make it tonight, I've got a deadline.")
e("spelling", "chat", "wont be long, just finishing teh report", "won't be long, just finishing the report")
e("spelling", "chat", "she didnt reply so im going to call her later", "she didn't reply so I'm going to call her later")
e("spelling", "chat", "thats weird, it was working yestarday", "that's weird, it was working yesterday")
e("spelling", "chat", "Are you comming to the party on saturday, its going to be fun?", "Are you coming to the party on saturday, it's going to be fun?")
e("spelling", "chat", "my freind was embarassed about the mistake", "my friend was embarrassed about the mistake")
e("spelling", "chat", "that was alot of work but it was sucessful", "that was a lot of work but it was successful")
e("spelling", "chat", "its realy noticable on the new screen", "it's really noticeable on the new screen")
e("spelling", "document", "The calender shows that the occassion falls on a Sunday.", "The calendar shows that the occasion falls on a Sunday.")
e("spelling", "document", "The company has a long existance and a strong enviroment policy.", "The company has a long existence and a strong environment policy.")
e("spelling", "document", "The independant review was publically released in the autumn.", "The independent review was publicly released in the autumn.")
e("spelling", "document", "Staff may aquire a parking permit through the maintainance office.", "Staff may acquire a parking permit through the maintenance office.")
e("spelling", "document", "She refered to the previous bussiness plan in her presentation.", "She referred to the previous business plan in her presentation.")
e("spelling", "document", "All employees have the privelege of flexible hours.", "All employees have the privilege of flexible hours.")
e("spelling", "document", "The writting on the sign was not clear to the collegue.", "The writing on the sign was not clear to the colleague.")
e("spelling", "notes", "buy bread, mlik and some fruit for tommorow", "buy bread, milk and some fruit for tomorrow")
e("spelling", "notes", "Remeber to call the plumber about the wierd noise.", "Remember to call the plumber about the weird noise.")
e("spelling", "notes", "Agenda: finish the proposel, review the budjet, plan the offsite.", "Agenda: finish the proposal, review the budget, plan the offsite.")
e("spelling", "notes", "Thier feedback was mostly positive but a few comments were harsh.", "Their feedback was mostly positive but a few comments were harsh.")
e("spelling", "notes", "Idea: write a short guide for new joiners and keep it simple becuase nobody reads long docs.",
  "Idea: write a short guide for new joiners and keep it simple because nobody reads long docs.")
e("spelling", "notes", "Learn the rythm of the song before the rehersal.", "Learn the rhythm of the song before the rehearsal.")
e("spelling", "code", "The funtion retuns early when the list is empty.", "The function returns early when the list is empty.")
e("spelling", "code", "Add a comment explaining why the timout is so long.", "Add a comment explaining why the timeout is so long.")
e("spelling", "code", "The test fails intermitently on the slower machine.", "The test fails intermittently on the slower machine.")
e("spelling", "code", "Remove the depricated option from the setings page.", "Remove the deprecated option from the settings page.")
e("spelling", "general", "The restaraunt was packed, so we waited outside.", "The restaurant was packed, so we waited outside.")
e("spelling", "general", "Its a beautful morning and the garden looks lovely.", "It's a beautiful morning and the garden looks lovely.")
e("spelling", "general", "We stayed at a hotel near the beach for a whole weeek.", "We stayed at a hotel near the beach for a whole week.")
e("spelling", "general", "He is allways early, which suprises everyone.", "He is always early, which surprises everyone.")
e("spelling", "general", "The grocery store has a good selction of fresh vegtables.", "The grocery store has a good selection of fresh vegetables.")

# ---------------------------------------------------------------- agreement
e("agreement", "email", "The results of the survey was very encouraging.", "The results of the survey were very encouraging.")
e("agreement", "email", "Our team have a clear plan for the next quarter, and each of us know our part.",
  "Our team has a clear plan for the next quarter, and each of us knows our part.")
e("agreement", "email", "There is several open issues that we need to discuss.", "There are several open issues that we need to discuss.")
e("agreement", "email", "He don't have access to the shared folder yet.", "He doesn't have access to the shared folder yet.")
e("agreement", "email", "The documents you requested is attached to this email.", "The documents you requested are attached to this email.")
e("agreement", "email", "Neither the manager nor the engineers was available on Friday.", "Neither the manager nor the engineers were available on Friday.")
e("agreement", "email", "The list of attendees have been updated.", "The list of attendees has been updated.")
e("agreement", "email", "She have already sent the signed contract.", "She has already sent the signed contract.")
e("agreement", "chat", "they was already gone when I arrived", "they were already gone when I arrived")
e("agreement", "chat", "We was thinking of ordering pizza tonight.", "We were thinking of ordering pizza tonight.")
e("agreement", "chat", "she don't know about the change yet", "she doesn't know about the change yet")
e("agreement", "chat", "Everyone are welcome to join the call.", "Everyone is welcome to join the call.")
e("agreement", "chat", "The new phones comes with a better camera.", "The new phones come with a better camera.")
e("agreement", "chat", "I is not sure what happened with the order.", "I am not sure what happened with the order.")
e("agreement", "document", "The data from the sensors are stored for thirty days.", "The data from the sensors is stored for thirty days.")
e("agreement", "document", "Each of the applicants have to submit a portfolio.", "Each of the applicants has to submit a portfolio.")
e("agreement", "document", "The policies applies to all full-time staff.", "The policies apply to all full-time staff.")
e("agreement", "document", "A number of changes was made to the original draft.", "A number of changes were made to the original draft.")
e("agreement", "document", "The quality of the images are poor.", "The quality of the images is poor.")
e("agreement", "document", "These kind of errors are easy to prevent.", "These kinds of errors are easy to prevent.")
e("agreement", "notes", "Both of the options looks fine to me.", "Both of the options look fine to me.")
e("agreement", "notes", "The meeting notes was shared with the whole team.", "The meeting notes were shared with the whole team.")
e("agreement", "notes", "Her sister and her cousin is coming for dinner.", "Her sister and her cousin are coming for dinner.")
e("agreement", "notes", "My laptop and phone needs charging before the trip.", "My laptop and phone need charging before the trip.")
e("agreement", "code", "The tests runs in about two minutes on the CI server.", "The tests run in about two minutes on the CI server.")
e("agreement", "code", "Each worker have its own queue.", "Each worker has its own queue.")
e("agreement", "general", "My parents lives in a small town near the coast.", "My parents live in a small town near the coast.")
e("agreement", "general", "The books on the shelf was all written by the same author.", "The books on the shelf were all written by the same author.")
e("agreement", "general", "There is two reasons why I changed my mind.", "There are two reasons why I changed my mind.")
e("agreement", "general", "He go to the gym every morning before work.", "He goes to the gym every morning before work.")

# ---------------------------------------------------------------- confusable
e("confusable", "email", "Please send me your updated address so I can confirm you're booking.", "Please send me your updated address so I can confirm your booking.")
e("confusable", "email", "I think your right about the deadline.", "I think you're right about the deadline.")
e("confusable", "email", "The team lost there files when the laptop crashed.", "The team lost their files when the laptop crashed.")
e("confusable", "email", "The new policy will effect all departments from January.", "The new policy will affect all departments from January.")
e("confusable", "email", "Everyone accept the interns signed the form.", "Everyone except the interns signed the form.")
e("confusable", "email", "The report is better then I expected.", "The report is better than I expected.")
e("confusable", "email", "Could you also send the invoice too the accounts team?", "Could you also send the invoice to the accounts team?")
e("confusable", "email", "I would of replied sooner if I had seen your message.", "I would have replied sooner if I had seen your message.")
e("confusable", "email", "Let me know whether or not the weather will delay the shipment, or weather we should reschedule.",
  "Let me know whether or not the weather will delay the shipment, or whether we should reschedule.")
e("confusable", "email", "Thanks for the advise, it was really useful.", "Thanks for the advice, it was really useful.")
e("confusable", "chat", "your going to love the new place", "you're going to love the new place")
e("confusable", "chat", "I think there coming at eight", "I think they're coming at eight")
e("confusable", "chat", "The cat licked its paws and then it's tail.", "The cat licked its paws and then its tail.")
e("confusable", "chat", "Its raining so take an umbrella.", "It's raining so take an umbrella.")
e("confusable", "chat", "dont loose the tickets", "don't lose the tickets")
e("confusable", "chat", "I'm to tired to cook tonight.", "I'm too tired to cook tonight.")
e("confusable", "chat", "Whose coming to the match on Sunday?", "Who's coming to the match on Sunday?")
e("confusable", "chat", "Their going to be late because of the traffic.", "They're going to be late because of the traffic.")
e("confusable", "document", "The principle of the school announced a new reading programme.", "The principal of the school announced a new reading programme.")
e("confusable", "document", "The effect of the change was larger then we had predicted.", "The effect of the change was larger than we had predicted.")
e("confusable", "document", "Employees must follow the safety principals set out in this document.", "Employees must follow the safety principles set out in this document.")
e("confusable", "document", "The two systems compliment each other well.", "The two systems complement each other well.")
e("confusable", "document", "Applications are accepted from all students accept those in the final year.", "Applications are accepted from all students except those in the final year.")
e("confusable", "notes", "I should of written this down yesterday.", "I should have written this down yesterday.")
e("confusable", "notes", "Check weather the file is saved before closing.", "Check whether the file is saved before closing.")
e("confusable", "notes", "Take a deep breathe and start again.", "Take a deep breath and start again.")
e("confusable", "notes", "The meeting was quiet short, we only discussed two points.", "The meeting was quite short, we only discussed two points.")
e("confusable", "code", "This change should not effect the existing tests.", "This change should not affect the existing tests.")
e("confusable", "code", "The function returns more then one value.", "The function returns more than one value.")
e("confusable", "general", "The dog wagged it's tail when she came home.", "The dog wagged its tail when she came home.")
e("confusable", "general", "She has less friends than her brother does.", "She has fewer friends than her brother does.")
e("confusable", "general", "We went to the park and than had ice cream.", "We went to the park and then had ice cream.")

# ---------------------------------------------------------------- article
e("article", "email", "I will send you a email with the details.", "I will send you an email with the details.")
e("article", "email", "She is an university lecturer in Chennai.", "She is a university lecturer in Chennai.")
e("article", "email", "We need a answer by the end of the day.", "We need an answer by the end of the day.")
e("article", "email", "The meeting will take a hour at most.", "The meeting will take an hour at most.")
e("article", "email", "Please attach the copy of your passport to application form.", "Please attach a copy of your passport to the application form.")
e("article", "email", "He is a honest person and a good manager.", "He is an honest person and a good manager.")
e("article", "email", "I am writing to apply for position of project manager.", "I am writing to apply for the position of project manager.")
e("article", "email", "It was an unique opportunity for the whole team.", "It was a unique opportunity for the whole team.")
e("article", "chat", "can you grab a umbrella from the hall", "can you grab an umbrella from the hall")
e("article", "chat", "I have a idea for the weekend.", "I have an idea for the weekend.")
e("article", "chat", "She bought new laptop yesterday, a really nice one.", "She bought a new laptop yesterday, a really nice one.")
e("article", "chat", "He wants to be a engineer when he grows up.", "He wants to be an engineer when he grows up.")
e("article", "document", "The company was founded in the 1998 by two friends.", "The company was founded in 1998 by two friends.")
e("article", "document", "This is a one-time payment, not an subscription.", "This is a one-time payment, not a subscription.")
e("article", "document", "The committee reviewed a proposal and approved an budget.", "The committee reviewed a proposal and approved a budget.")
e("article", "document", "Employees receive a hour of paid training every week and a half day on Fridays.", "Employees receive an hour of paid training every week and a half day on Fridays.")
e("article", "notes", "Need to buy a apple and an orange for the lunch box.", "Need to buy an apple and an orange for the lunch box.")
e("article", "notes", "Write short summary of the call before the Friday.", "Write a short summary of the call before Friday.")
e("article", "general", "She is learning to play a violin and an piano.", "She is learning to play a violin and a piano.")
e("article", "general", "It was an very long journey, but the view was worth it.", "It was a very long journey, but the view was worth it.")

# ---------------------------------------------------------------- punctuation
e("punctuation", "email", "Hi Priya thanks for sending the files.", "Hi Priya, thanks for sending the files.")
e("punctuation", "email", "When will the package arrive.", "When will the package arrive?")
e("punctuation", "email", "I tried calling you yesterday , but your phone was off.", "I tried calling you yesterday, but your phone was off.")
e("punctuation", "email", "Dear Mr. Rao ,", "Dear Mr. Rao,")
e("punctuation", "email", "We received Sarahs application and Johns reference.", "We received Sarah's application and John's reference.")
e("punctuation", "email", "I called the supplier He said the order would ship on Monday.", "I called the supplier. He said the order would ship on Monday.")
e("punctuation", "email", "What time should we meet on Friday", "What time should we meet on Friday?")
e("punctuation", "email", "Thanks again , and have a great weekend .", "Thanks again, and have a great weekend.")
e("punctuation", "chat", "are you coming tonight", "are you coming tonight?")
e("punctuation", "chat", "ok thanks , see you then", "ok thanks, see you then")
e("punctuation", "chat", "Hey Meera are you free for a quick call?", "Hey Meera, are you free for a quick call?")
e("punctuation", "chat", "It's Divyas birthday on Friday.", "It's Divya's birthday on Friday.")
e("punctuation", "chat", "Where did you put the keys.", "Where did you put the keys?")
e("punctuation", "document", "The meeting is scheduled for Monday ; all staff must attend.", "The meeting is scheduled for Monday; all staff must attend.")
e("punctuation", "document", "The companys annual report is available on request.", "The company's annual report is available on request.")
e("punctuation", "document", "Please bring your passport , a photo and proof of address.", "Please bring your passport, a photo and proof of address.")
e("punctuation", "document", "The results were clear We will publish them next week.", "The results were clear. We will publish them next week.")
e("punctuation", "document", "The childrens library opens at ten.", "The children's library opens at ten.")
e("punctuation", "notes", "Todo : call the bank , book the tickets.", "Todo: call the bank, book the tickets.")
e("punctuation", "notes", "Why did the build fail again", "Why did the build fail again?")
e("punctuation", "notes", "Ask Karthik about the budget He knows the numbers.", "Ask Karthik about the budget. He knows the numbers.")
e("punctuation", "notes", "Pick up milk , eggs and bread.", "Pick up milk, eggs and bread.")
e("punctuation", "code", "The function fails when the list is empty ; check for that first.", "The function fails when the list is empty; check for that first.")
e("punctuation", "general", "My brothers car is in the garage.", "My brother's car is in the garage.")

# ---------------------------------------------------------------- doubled_word
e("doubled_word", "email", "Please send the the report by noon.", "Please send the report by noon.")
e("doubled_word", "email", "I wanted to to thank you for the quick reply.", "I wanted to thank you for the quick reply.")
e("doubled_word", "email", "The invoice is is attached below.", "The invoice is attached below.")
e("doubled_word", "email", "We will review the the proposal and get back to you.", "We will review the proposal and get back to you.")
e("doubled_word", "email", "Thanks for your your help with the event.", "Thanks for your help with the event.")
e("doubled_word", "email", "Let me know if you need anything anything else.", "Let me know if you need anything else.")
e("doubled_word", "chat", "i think the the meeting was moved", "i think the meeting was moved")
e("doubled_word", "chat", "can you can you send me the link", "can you send me the link")
e("doubled_word", "chat", "That was was a great game.", "That was a great game.")
e("doubled_word", "chat", "I am am on my way.", "I am on my way.")
e("doubled_word", "chat", "see you at at the station", "see you at the station")
e("doubled_word", "document", "The report was was submitted on time.", "The report was submitted on time.")
e("doubled_word", "document", "All staff must must wear a badge.", "All staff must wear a badge.")
e("doubled_word", "document", "The policy applies to to all full-time staff.", "The policy applies to all full-time staff.")
e("doubled_word", "document", "The The committee will meet on Tuesday.", "The committee will meet on Tuesday.")
e("doubled_word", "notes", "Remember to to book the tickets.", "Remember to book the tickets.")
e("doubled_word", "notes", "Call the the dentist on Tuesday.", "Call the dentist on Tuesday.")
e("doubled_word", "notes", "Ideas for for the offsite: a hike or a cooking class.", "Ideas for the offsite: a hike or a cooking class.")
e("doubled_word", "code", "The function returns returns early when the list is empty.", "The function returns early when the list is empty.")
e("doubled_word", "general", "She said that that she was tired.", "She said that she was tired.")

# ---------------------------------------------------------------- capitalisation
e("capitalisation", "email", "thanks for the update. i will review it tonight.", "Thanks for the update. I will review it tonight.")
e("capitalisation", "email", "Hello team. the release is scheduled for friday.", "Hello team. The release is scheduled for Friday.")
e("capitalisation", "email", "We meet again in march, and the next review is in june.", "We meet again in March, and the next review is in June.")
e("capitalisation", "email", "I spoke to the client on monday and again on wednesday.", "I spoke to the client on Monday and again on Wednesday.")
e("capitalisation", "email", "please find the document attached. let me know if anything is missing.", "Please find the document attached. Let me know if anything is missing.")
e("capitalisation", "email", "My colleague speaks english, hindi and telugu fluently.", "My colleague speaks English, Hindi and Telugu fluently.")
e("capitalisation", "email", "i am travelling next week, so i will reply when i am back.", "I am travelling next week, so I will reply when I am back.")
e("capitalisation", "email", "Good morning. we have moved the call to tuesday at 11.", "Good morning. We have moved the call to Tuesday at 11.")
e("capitalisation", "chat", "i think we should go now", "I think we should go now")
e("capitalisation", "chat", "ok i'll be there in ten", "ok I'll be there in ten")
e("capitalisation", "chat", "yes i know, i'm sorry", "yes I know, I'm sorry")
e("capitalisation", "chat", "see you on saturday!", "see you on Saturday!")
e("capitalisation", "chat", "I told her i would call back", "I told her I would call back")
e("capitalisation", "chat", "the meeting is on thursday not wednesday", "the meeting is on Thursday not Wednesday")
e("capitalisation", "document", "the committee approved the plan. the budget will follow in the autumn.", "The committee approved the plan. The budget will follow in the autumn.")
e("capitalisation", "document", "The course is taught in english and runs from september to december.", "The course is taught in English and runs from September to December.")
e("capitalisation", "document", "applicants should apply before the first of april. late forms will not be accepted.", "Applicants should apply before the first of April. Late forms will not be accepted.")
e("capitalisation", "document", "The office is closed on sundays and public holidays.", "The office is closed on Sundays and public holidays.")
e("capitalisation", "notes", "call the bank on monday. then book the tickets.", "Call the bank on Monday. Then book the tickets.")
e("capitalisation", "notes", "reminder: i need to renew the passport before october.", "Reminder: I need to renew the passport before October.")
e("capitalisation", "notes", "ideas for the trip. we could visit the lake, the old fort and the market.", "Ideas for the trip. We could visit the lake, the old fort and the market.")
e("capitalisation", "code", "the parser is fast. it only allocates once per file.", "The parser is fast. It only allocates once per file.")
e("capitalisation", "general", "the train leaves at noon. we should hurry.", "The train leaves at noon. We should hurry.")
e("capitalisation", "general", "She was born in july and moved to Bangalore in december.", "She was born in July and moved to Bangalore in December.")

# ---------------------------------------------------------------- indian_english (typed-text patterns that have a plain standard form)
e("indian_english", "email", "Kindly revert back at the earliest.", "Kindly reply at the earliest.", bad=["revert back"])
e("indian_english", "email", "Please revert back once the deployment is completed.", "Please reply once the deployment is completed.", bad=["revert back"])
e("indian_english", "email", "I am having a doubt about the second clause of the agreement.", "I have a doubt about the second clause of the agreement.")
e("indian_english", "email", "We have been waiting since three days for the approval.", "We have been waiting for three days for the approval.")
e("indian_english", "email", "Let us discuss about the budget on Monday.", "Let us discuss the budget on Monday.")
e("indian_english", "email", "Please do not hesitate to contact me, I will return back to you tomorrow.", "Please do not hesitate to contact me, I will get back to you tomorrow.", bad=["return back"])
e("indian_english", "email", "Could you explain me the process for claiming the reimbursement?", "Could you explain to me the process for claiming the reimbursement?")
e("indian_english", "email", "I am not able to cope up with the workload this month.", "I am not able to cope with the workload this month.", bad=["cope up"])
e("indian_english", "email", "The new vendor is more better than the previous one.", "The new vendor is better than the previous one.", bad=["more better"])
e("indian_english", "email", "I am knowing the client for many years.", "I have known the client for many years.")
e("indian_english", "email", "He is having two laptops, one for office and one for home.", "He has two laptops, one for the office and one for home.")
e("indian_english", "email", "We should emphasize on quality rather than speed.", "We should emphasize quality rather than speed.")
e("indian_english", "email", "Please enquire about the status and inform me the same.", "Please enquire about the status and inform me of the same.")
e("indian_english", "email", "I am understanding the issue now, thank you.", "I understand the issue now, thank you.")
e("indian_english", "email", "He can able to finish the report by Friday.", "He is able to finish the report by Friday.")
e("indian_english", "chat", "I am having fever since yesterday so I will not come.", "I have had a fever since yesterday so I will not come.")
e("indian_english", "chat", "we are discussing about the plan right now", "we are discussing the plan right now")
e("indian_english", "chat", "please revert back if you are free in the evening", "please reply if you are free in the evening", bad=["revert back"])
e("indian_english", "chat", "He is staying here since 2019.", "He has been staying here since 2019.")
e("indian_english", "chat", "I am wanting to talk to you about something.", "I want to talk to you about something.")
e("indian_english", "chat", "She told me that she will call me yesterday.", "She told me that she would call me yesterday.")
e("indian_english", "chat", "I am liking the new phone very much.", "I like the new phone very much.")
e("indian_english", "chat", "Can you explain me how this works?", "Can you explain to me how this works?")
e("indian_english", "document", "The manager will discuss about the new policy in the town hall.", "The manager will discuss the new policy in the town hall.")
e("indian_english", "document", "The employees are requested to return back the laptops by Friday.", "The employees are requested to return the laptops by Friday.", bad=["return back"])
e("indian_english", "document", "The results are more better than last year's.", "The results are better than last year's.", bad=["more better"])
e("indian_english", "document", "We are having a meeting since morning.", "We have been having a meeting since morning.")
e("indian_english", "document", "The students are knowing the answer.", "The students know the answer.")
e("indian_english", "document", "All members should cope up with the new schedule.", "All members should cope with the new schedule.", bad=["cope up"])
e("indian_english", "notes", "Discuss about the vendor contract with Rohan.", "Discuss the vendor contract with Rohan.")
e("indian_english", "notes", "I am having a question for the review meeting.", "I have a question for the review meeting.")
e("indian_english", "notes", "Revert back to the client on the pricing.", "Reply to the client on the pricing.", bad=["revert back"])
e("indian_english", "notes", "She is working here since two years.", "She has been working here for two years.")
e("indian_english", "notes", "He is having good knowledge of the system.", "He has good knowledge of the system.")
e("indian_english", "code", "We are discussing about the retry logic in the review.", "We are discussing the retry logic in the review.")
e("indian_english", "general", "I am not understanding what he wants.", "I do not understand what he wants.")
e("indian_english", "general", "My cousin is living in Mumbai since 2020.", "My cousin has been living in Mumbai since 2020.")


# ---------------------------------------------------------------- build
PROT = re.compile(r"https?://\S+|www\.\S+|\S+@\S+\.\S+|[A-Za-z]:\\\S+|%\w+%\S*|\S*/\S+|\S*\\\S+|(?<!\S)--?[A-Za-z][\w-]*"
                  r"|\S*\d\S*|\b\w+_\w+(?:::\w+)*\b|\b[a-z]+[A-Z]\w*\b|\b[A-Z]{2,}\b|\b\w+\.\w{1,5}\b")
WORD = re.compile(r"[A-Za-z']+")
PREFIX = dict(spelling="sp", agreement="ag", confusable="cf", article="ar", punctuation="pu", doubled_word="dw",
              capitalisation="ca", indian_english="ie", clean_everyday="ce", clean_technical="ct", protected="pr")


def protected_tokens(inp, ref):
    found = []
    for m in PROT.finditer(ref):
        tok = m.group().lstrip("(\"'`").rstrip(".,;:!?)\"'`")
        if tok and tok in inp and tok not in found:
            found.append(tok)
    return found


def build():
    rows, seen = [], set()
    counter = Counter()
    for cat, ctx, inp, ref, keep, bad in CASES:
        assert cat in CATEGORIES and ctx in CONTEXTS, (cat, ctx, inp)
        assert (inp == ref) == cat.startswith("clean_") or cat == "protected", ("clean/error mismatch", inp)
        assert inp not in seen, ("duplicate input", inp)
        seen.add(inp)
        keep = list(dict.fromkeys(list(keep) + protected_tokens(inp, ref)))
        if cat == "spelling":
            ref_words = {w.lower() for w in WORD.findall(ref)}
            bad = list(bad) + [w.lower() for w in WORD.findall(inp) if len(w) >= 4 and w.lower() not in ref_words
                               and w.lower() not in ref.lower()]
        for k in keep:
            assert k in inp and k in ref, ("must_keep not in input and reference", k, inp)
        for b in bad:
            assert b.lower() in inp.lower() and b.lower() not in ref.lower(), ("must_not_contain", b, inp)
        counter[cat] += 1
        rows.append({"id": f"{PREFIX[cat]}_{counter[cat]:02d}", "category": cat, "context": ctx, "input": inp,
                     "reference": ref, "must_keep": keep, "must_not_contain": list(dict.fromkeys(bad))})
    return rows


def main():
    rows = build()
    n = Counter(r["category"] for r in rows)
    assert len(rows) >= 300 and n["clean_everyday"] >= 60 and n["clean_technical"] >= 60 and n["protected"] >= 40, n
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"wrote {len(rows)} cases to {OUT}")
    print("  by category: " + ", ".join(f"{c}={n[c]}" for c in CATEGORIES))
    ctx = Counter(r["context"] for r in rows)
    print("  by context: " + ", ".join(f"{c}={ctx[c]}" for c in CONTEXTS))


if __name__ == "__main__":
    main()

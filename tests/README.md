# VOLT tests

    node tests/run.js            # everything
    node tests/run.js syntax     # the no-dependency one
    node tests/run.js arp lpb ui

**`syntax`** needs nothing but node. It parses every inline script in `volt.html` and `beta.html`
and checks the JSON kits still parse. Run it before every push — a syntax error on `main` means a
blank page for anyone who opens the site.

**`lpb`, `arp`, `ui`** drive a real headless Chrome. One-time setup:

    npm i -D puppeteer-core

They find Chrome in the usual Windows/mac/Linux spots; set `CHROME_PATH` if yours is elsewhere.
Without them the runner skips those suites and says so — it never silently passes.

By default the suites test **beta.html**. To check the stable file:

    VOLT_FILE=volt.html node tests/run.js

## What's actually proved here

Most of these don't just inspect data — `lpb` and `arp` **render the song through VOLT's own
offline WAV path and measure where the notes really start**. That's how "changing rows/beat doesn't
change how the song sounds" and "triplets are really triplets" are verified instead of assumed.

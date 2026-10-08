# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **The Windows build carries the default config with the same line endings
  as every other build.** It used to embed the file as the build machine's
  checkout wrote it, which on Windows is CRLF, so `--print-config` and the
  first-run config differed from macOS and Linux for no reason in the code.
  The repository now asks for LF everywhere. A config already on disk is
  untouched, and edits keep whatever endings it has.
- **Every dialog punches its title into the border.** The `w` picker's
  `PANELS` and a prompt's label — `ADD A CLOCK`, `WEATHER LOCATION`,
  `AGENDA FILE` — were laid on the border bare, where the theme picker, the
  key map and every panel draw `┤TITLE├`. All four dialogs now draw it the
  same way, and one too narrow for its title cuts it with `…` rather than
  letting the corner take the closing `├`.

### Fixed

- **An absurd pomodoro length is refused instead of overflowing.** The three
  `[pomodoro]` lengths were checked only for being above zero, and the panel
  turns minutes into seconds unchecked, so a hand-edited length too large
  for that sum crashed a debug build and gave a release build a phase of
  some other length nobody wrote. Like the two refresh settings, each now
  stops at a year, and a config past it is refused with the key named.
- **`--migrate-config` no longer overwrites an earlier backup.** It copied the
  original to `config.toml.bak` whether or not that name was taken, so the
  config a `--reset-config` had set aside there, or an earlier migration's
  backup, was replaced for good. It now takes the next free number, as the
  resets do.
- **The error for a key from an older version points at `--migrate-config`
  exactly when the migration would fix the file.** The error kept its own list
  of those keys, and the list had three of the four the migration fixes, so a
  pre-0.1.0 `[notes] side_by_side_min_width` was told to read `--print-config`
  instead. It went by the key's name alone, where the migration also needs
  the right table, so `forecast_days` under `[clocks]` was sent to a migration
  that then refused it. And it promised an update in place wherever the key
  matched, though the migration writes nothing when the result would still
  not load: a `forecast_hours` added beside the stale `forecast_days` was
  refused as a duplicate. The error now asks the migration about the line the
  parser stopped on and whether it would finish. When it would, the error says
  what it will change; when it would not, it says what the line becomes and
  the first problem that stops the migration, by line.
- **The theme picker draws the theme it is previewing on a short terminal.**
  Its list kept a twelve-row window in a dialog sixteen rows tall, so on a
  terminal under sixteen rows the foot of the window was cut, footer first,
  and `End` or `↓` previewed a theme whose name was nowhere on screen. The
  list now scrolls within the rows the terminal leaves it, as the `w`
  picker's does, the footer gives way to nothing but the row of the theme
  being previewed, and a page moves as far as the rows drawn.
- **A dialog's keys drop whole on a narrow terminal, and Esc goes last.**
  The `w` and `t` pickers drew their key rows at full width and the terminal
  cut them wherever the edge fell — `Esc put` with no `back`, and narrower
  still the keep hint whole and Esc gone without a mark. The key map's row
  dropped from the end, so a narrow one said how to reload and not how to
  leave, and a prompt on a screen under twenty columns measured its help
  for the twenty it asked for. Every dialog's keys now drop whole, the way
  out last, and Esc on its own and still too wide ends in `…`, as a
  prompt's help already did.
- **The `w` picker says when it has cut a message.** The line under its list
  gives way to what was just refused or why the config could not be
  written, and most of those are longer than the dialog's forty columns —
  `the edited config does not describe the requested layout` — so the
  terminal cut them at every size, with nothing to show the rest was
  missing. They now end in `…`, and so does the usual line on a narrow
  terminal.
- **One refresh of the watchlist asks for each symbol once.** An `r`, or an
  added symbol, while the markets panel waited out the minute it leaves
  between rounds was answered by the next round and then again a minute
  later, so it cost two requests a symbol against a source that blocks by
  address. Removing a symbol also threw away an `r` pressed
  just before it, and the board waited out the whole interval instead.
- **The markets panel copies its board when a price lands, not every
  frame.** It cloned every quote with its intraday series once a second, for
  numbers that change once a minute, and redrew each sparkline from the
  whole series. Both happen when a quote lands now. A quote also keeps at
  most a thousand intraday prices, averaged down rather than cut: the
  request asks for 78, and nothing stopped an answer carrying two million.
- **A long task or note list builds only the rows on screen.** Every task
  and every note in the list was turned into a row on every frame, through
  an index of the whole store rebuilt each time, for a panel that drew the
  twenty that fit. Scrolling, the selection and clicks are as they were.
- **The task form's placeholders say when they are cut.** In a form narrower
  than about 27 columns, `what needs doing` was cut by the terminal to a
  whole-looking `what needs d`. The placeholders now end in `…` where they
  do not fit.
- **The disk panel's read and write rates are in the units of its
  capacity.** The readout under each device borrowed the network panel's
  figures, which count a kilobyte as 1,024 bytes, beside free space and size
  counted in thousands, the way a disk is sold — so one device showed two
  meanings of `MB`, and a disk moving a hundred million bytes a second read
  `95.4 MB/s`. The rates are decimal now, with a decimal place below ten of
  a unit and whole figures above: `100 MB/s`, `1.4 MB/s`, `327 kB/s`, and
  `0 kB/s` for an idle disk.
- **A temperature panel with no sensors marks the cut in its headline.**
  Narrower than `No temperature sensors`, it showed as much of the words as
  fitted — `No temperatu` — with nothing to say the rest had gone. It ends
  in `…` now, as the disk and battery panels' empty states already did.
- **The shipped config and the README name every widget there is.** The
  README's guide to the panels opened on "Fourteen widgets", and the
  config's `Available widgets` comment stopped at thirteen and left out
  `memory`, `disk`, `battery` and `temperature` — three of them the panels
  the default layout does not place, which a reader adding one by hand has
  to find by name. The README says seventeen, and the comment
  lists all seventeen now, in a new config and in `--print-config`; a config
  already on disk keeps the comment it was written with.
- **Adding a clock on a short terminal adds the city on screen.** The zone
  list kept a ten-row window in a dialog fifteen rows tall, so on a shorter
  terminal its foot was cut, `Esc cancels` first, while `↓` walked on into
  rows nobody could see and Enter added one of them. The list now scrolls
  within the rows the terminal leaves it, as both pickers' do, a page moves
  as far as the rows drawn, and the help with the way out gives way to
  nothing but the field and the city under the cursor. On a terminal too
  short to draw that city, Enter waits rather than adding it; Esc still
  cancels.
- **An empty panel's notice says when its lower lines are cut.** The disk,
  battery and temperature panels explain an empty face in a line or two,
  and a panel too short for all of them drew the first with nothing to say
  the rest was missing. The last line drawn ends in `…` now when a line
  below it had something to say.
- **A line cut through emoji is no wider than its room.** An emoji written
  with its presentation selector, `❤️` or `☀️`, or joined to another, `👩‍💻`,
  is drawn two cells wide, and the cut that ends a long title or headline in
  `…` measured it a character at a time, as one. Each such emoji left the
  line a cell wider than its room, and the terminal cut the excess without
  a mark. The cut now measures them as they are drawn.
- **A prompt's caret stays after the text once the field is full.** Typing
  past the width of a prompt's field — a long path to the agenda file, say —
  scrolls the text, and the caret stopped a cell short of the end and sat
  on the last character typed, hiding the one just entered. It now sits in
  the cell after the text, which the field always kept free for it.
- **The key map's footer names the keys its reset question takes.** While
  `d` asked whether to put every key back, the footer still offered
  `r reload`, `d defaults` and `Esc close`, and each of them answered no
  instead. It reads `y reset  Esc keep` until the question is answered.
- **A paste takes the update notice down at once.** A paste retires the
  notice and marks the watch log read, as a key does, but the screen is
  redrawn after a paste only when something asks, and a paste the focused
  panel did not take asked for nothing. The notice, and a watch log rule
  line the paste had just made untrue, stayed up until something else
  redrew. Both now go with the paste, as they do with a click.

## [1.20.0] - 2026-10-07

### Changed

- **A setting given a word it does not take now stops startup and says what
  to write.** `[todo].sort`, `[notes].preview` and `[calendar].week_starts`
  each take one of a few words, and a misspelt one — `sort = "dues"`,
  `preview = "besides"`, or `week_starts = "Monday "` with its trailing
  space — used to start the dashboard on the default without a word, so the
  setting looked broken. mirador now refuses the config, names the key and
  lists the words it takes, as it already did for the two `units` keys. Case
  still does not matter for these three, so every value a panel actually
  read still loads; only a word it was ignoring is refused.
- **The test suite no longer reaches the network or your data directory.** It
  is documented as never touching the network, and that had stopped being true
  a second time: the weather and news panels start a fetch thread as they are
  built, and two tests built every widget, so each run asked Open-Meteo for a
  forecast and read the three shipped news feeds. Those tests, and almost
  every one that built a dashboard, also found the task list, notes,
  watchlist and world clocks in mirador's own data directory — reading your
  tasks, and on a machine that had none of the others yet, writing them there
  from the default config, after which your `[clocks].zones` and
  `[stocks].symbols` would never have been read, since a list seeds only
  while its file is missing. Under `cfg(test)` the three panels with a fetch
  thread are refused rather than built, the tests build them offline, and the
  data directory is one of the test run's own. No effect on the shipped
  binary.
- **A charging or full battery stays brass.** The battery panel painted its
  label, its cell and its figure green whenever the laptop was charging or
  full, where the README and the panel's own notes said brass while there is
  plenty, the signal colours only for a charge that is low while the machine
  runs on it, and charging told by the label. The face now does what they
  say: `CHARGING` and `FULL` name the state, in brass, so plugging in no
  longer turns the panel green. Amber and red are unchanged.

### Fixed

- **A symlinked config or data file stays a symlink.** A save writes a new
  file and renames it into place, and a rename replaces whatever has the
  name — so a `config.toml` linked into a dotfiles repository, or a task list
  linked into a synced folder, was turned into a plain file by the first
  change, and the copy the link pointed at silently stopped receiving
  anything after it. Saves now go to the file at the end of the link, and a
  link to a file that does not exist yet creates that file — but not the
  folder it would be in, which is reported instead, so a link into a sync
  folder not yet set up never gets a default file put where the real one is
  about to arrive. A loop of links is refused with an error rather than
  replaced. A config linked into a read-only place, as Nix home-manager does,
  is no longer replaced by a plain file either, so a layout change there now
  reports that it could not be saved, reason first. `--reset-config` moves a
  linked config aside as the link itself and writes the defaults to a new
  file, so the file the link points at is left as it was. Both resets set a
  link aside even when its file is missing, and a backup that is such a link
  is never taken for a free name, so a later reset does not write through it.
- **A failed save no longer leaves a temporary file behind.** Each save is
  written to a temporary under a name of its own first, and when the write or
  the rename failed it stayed there. A failed save is tried again at the next
  change, so a full disk, or a task list held open by another program on
  Windows, gathered one more `.tmp` file beside your data for every attempt.
- **A file you restricted is never written anywhere wider.** The mode of a
  `chmod 600` task list was copied onto the replacement only after the whole
  list had been written into it and flushed, so for that moment it sat in a
  file the umask had made readable by every account on the machine. The
  replacement is now created with the original's mode before anything is
  written into it.
- **`[todo].horizon_days` hides what it says it hides.** It has been in every
  config since the first release, described as hiding tasks due further out
  than that many days, and nothing read it: a list set to a week still showed
  tasks due next year. A task due more than that many days from today is now
  hidden, and comes back on the day it comes within range. A task with no due
  date, or one already late, always shows, and `0` still shows everything.
  Saving a task past the horizon says so, the summary line says how many
  tasks are further out, and a list the horizon has emptied says so rather
  than "No tasks yet". A filter searches past the horizon, so a task given a
  due date further out than meant can still be found, edited and deleted.
- **`[cpu].warn_pct` and `critical_pct` are documented as not read.** The
  shipped config set both, promising a readout that turned amber and red at
  those figures, and nothing ever did: the readout takes its colour from the
  graph's ramp, so it warms with the load instead. A config that sets them
  still loads. A new config leaves them out.
- **A paste of several lines into a one-line form adds one task and nothing
  else.** A newline in a paste is Enter, and Enter saves the task, after
  which the rest of the paste reached the task list as commands: pasting
  `Buy milk` and `and eggs` added "Buy milk", then `a` opened a new task and
  "nd eggs" became a second one, and a line beginning with a space or `dy`
  would have marked a task done or deleted it. The paste now stops where the
  form closes. The stocks panel's symbol prompt had the same fault.
- **A paste no longer answers a delete confirmation.** With a task, a note
  or a symbol waiting on `y` to be deleted, a paste was typed in as keys,
  so one beginning with `y` deleted it. The question now ignores a paste and
  waits for a key.
- **Browsing themes no longer remembers them.** Every arrow key in the `t`
  picker wrote the theme under the cursor to `state.toml`, so a dashboard
  closed mid-browse opened next time in a theme nobody chose. Only Enter
  records one now.
- **Esc in arrange mode undoes a resize made in it.** A `Ctrl+arrow` resize
  inside the mode was written to the config three quarters of a second
  later, so Esc put the widths back on screen and the next launch brought
  the resize back. Resizes made in the mode are now kept or discarded with
  the rest of the arrangement.
- **Refusing to switch off the last panel no longer says the layout could
  not be saved.** The refusal was reported as a failed save on the status
  bar, where it stayed after the picker closed until some later layout
  change succeeded. It is said in the picker, and goes when the picker
  does.
- **A layout change that cannot be saved as mirador quits is reported.**
  A resize made just before `q` is written on the way out, and when that
  write failed nothing said so. mirador now prints why once the terminal is
  restored.
- **Clicks and the mouse wheel stay out of panels behind an open dialog.**
  With the panel picker, the theme picker or the key map open, a click
  moved focus to the panel beneath it and selected a row nobody could see,
  and the wheel scrolled lists behind it. The dialogs take the mouse the
  way they take the keyboard.
- **"Every weekday" no longer meets at weekends.** Thunderbird writes it as a
  daily rule naming Monday to Friday, and the agenda read the weekdays only
  on weekly rules, so it showed the meeting on Saturday and Sunday too. A
  daily rule now falls only on the days it names. A daily or weekly rule
  naming weekdays also shows the day it was set up, which the standard
  counts as the first occurrence whatever the rule names. A weekly rule that
  skips weeks groups its days by the week start it gives, so a fortnightly
  Sunday-and-Monday meeting from a calendar whose weeks start on Sunday
  lands on the right pair. A monthly or yearly rule
  naming weekdays, which means every such weekday of the month or year, is
  outside what the agenda reads and shows only its first occurrence, rather
  than one on the same date each month whatever day that was.
- **A repeating event's last day is no longer lost west of Greenwich.** An
  end date written without a time, as Google writes the end of an all-day
  series, or a time without a zone, was read as UTC, so in the Americas the
  series ended the evening before its last day. Both are now read on the
  event's own clock and include that day, wherever you are. A broken time in
  the end date, which falls back to the date it carries, now keeps that day
  too.
- **A weekly event on several days keeps the early ones at the end of the
  agenda.** A rule meeting on Mondays and Wednesdays that began on a
  Wednesday dropped the Monday on the agenda's last day.
- **A daily event set up more than about eleven years ago appears again,**
  if it repeats without a set number of occurrences. Expanding a rule walked
  from its first occurrence and gave up after four thousand steps, so an old
  daily reminder never reached today and vanished without a word. Such a rule
  now starts just short of the days on show. One with a count still walks
  from the start, since it has to count, and four thousand steps remains its
  reach.
- **A repeating all-day event ends at midnight on the day the clocks
  change.** Its length was measured in hours, so the occurrence on a 23-hour
  day ran to 01:00 the next morning, still in progress, and the one on a
  25-hour day ended at 23:00.
- **One enormous event title no longer slows the dashboard.** A calendar's
  title and location are drawn on every frame, and line folding let either
  run to the whole 10MB the agenda reads; both are now kept to 400
  characters, far more than the panel shows.
- **The watch log no longer reports tomorrow's meetings as new every
  midnight.** The agenda reads a window of days starting today, so each
  midnight brings in a day the previous read never looked at, and every
  event on it, including the next instance of every repeating meeting, was
  logged as having "appeared in your calendar". Only an event inside the
  previous window is reported now: one past its end was out of view, not
  missing.
- **The agenda's `o` shows the whole path.** The calendar's path was drawn
  as one line and the terminal cut it at the panel's edge, so the default
  macOS path lost its filename at every width the panel takes, with no `…`
  to say so. It wraps now, and a panel too short for all of it ends the
  last row it has in `…`.
- **The meeting under way keeps its location.** The `▸` beside it was
  counted as four cells, its size in bytes, rather than two, so that row
  cut its summary two cells early and dropped a location the rows around
  it kept.
- **The world clocks write `zones.toml` on the first run.** The list from
  `[clocks].zones` was meant to be saved on the first run, but it was only
  saved after a clock was changed from the panel. Until then the file did
  not exist, `o` named a missing file, and the config was read again at
  every launch, though the shipped config says it is read once. After the
  first change, edits to the config stopped working with nothing to say
  why. The list is now saved when the panel first starts, as the stock
  watchlist is. If you have been editing `[clocks].zones` in your config,
  this release copies it into `zones.toml` on first start, and from then
  on you edit `zones.toml` or use the panel.
- **A clock table taller than its panel scrolls with the cursor.** Clocks
  past the bottom of the panel were cut off with nothing to mark them, and
  the cursor could move onto one that was not drawn, where `d` would remove
  a clock you could not see. The table now shows the part that holds the
  selected clock. A message such as `o`'s path has a row of its own,
  taken from spare room where there is any, instead of covering the last
  clock, and `the big clock stays` now appears when there is no table
  under the clock to show it with.
- **The `ansi` and `high-contrast` themes colour their graphs again.** A
  gradient written with colour names was blended through a stand-in grey,
  so every graph, meter and gain or loss figure in both themes came out one
  true-colour grey whatever the load — sent as a 24-bit escape by the theme
  that exists to avoid them. A ramp of named or numbered colours now steps
  between the colours it names, so `ansi` runs green, yellow, red in your
  terminal's own palette, and hex ramps blend as before.
- **A fast disk's I/O graph is drawn at the right height.** Above about two
  gigabytes a second the graph's scale was capped while the readings were
  not, so a 3 GB/s read on a disk peaking at 5 GB/s filled seventy per cent
  of the graph instead of thirty, and anything past 4.3 GB/s drew as a full
  column.
  The disk panel's combined graph reached this on an ordinary large copy
  from a modern SSD.
- **`L` no longer renames a weather panel that `[weather]` pins with
  coordinates.** `latitude` and `longitude` win over the place name, so a
  name typed at `L` only retitled the panel over the old place's weather,
  and was remembered that way across restarts. With coordinates configured
  `L` now says to remove them to choose a place by name, and a place
  remembered before they were added no longer relabels them. A name you gave
  coordinates with `L` in an earlier version is dropped the same way; put it
  in `[weather].location` to keep it.
- **"City, Country" finds the city in that country.** The text after the
  comma was matched against the region and the two-letter country code
  only, so `London, Canada` — the form the `L` prompt suggests — showed
  London, England. The country's name now counts too.
- **A failing weather refresh says why.** Once a reading was on screen a
  failure showed only "refresh failing", so a misspelt place left the old
  place's weather up and never said the new one was not found. The reason
  now follows the reading's age, cut with `…` where the panel is narrow.
- **The weather panel bounds what the weather service sends it.** A place
  name, region or observation time is drawn on every frame, and a broken or
  hostile response could make any of them megabytes long; a failure's
  reason could too, because the JSON parser quotes the value it rejects.
  Names and reasons are now cut at parse with `…`, far above anything real,
  and an observation time that is not a time is dropped.
- **An edited note's date can be read.** The mark that says a note was
  edited took the date column one cell past its width, so at the shipped
  date format every edited note read `·25 J…`. The column is a cell wider
  and the date is whole.
- **The notes panel marks what it cuts.** A long title in the note pane,
  a long search term above the list, and an error in the note form were
  each drawn at their full length and cut wherever the panel's edge fell.
  Each now ends in `…` where it does not fit, and the form's keys drop
  whole rather than being cut in half, `Esc cancel` the last to go.
- **The task form shows its whole hint, its whole error and its way out.**
  The row under the fields was one row high and drew only the first line of
  what it was given, so the Due hint lost `or empty.` and a date it could
  not read lost the list of forms it would take. The message now has the
  rows the form can spare, and ends in `…` when even those are not enough.
  The key row below it was cut to `Esc can` on the default dashboard; its
  keys now drop whole, `Esc cancel` the last to go.
- **The task filter offers whole tags.** With nothing typed, the filter bar
  lists the tags in use, and on a narrow panel the last of them was cut to
  something like `#mira`, a tag that does not exist. Tags that do not fit
  are left off.
- **A prompt's help keeps `Esc cancels`.** The help line under a prompt was
  cut at the dialog's edge, and two of them never fitted: the weather
  location's and the add-a-clock prompt's lost the end of `Esc cancels` at
  every terminal size. Both are shorter, and a help line that does not fit
  now drops whole parts, the way out last.
- **The calculator shows the end of a long sum as it is typed.** An entry
  wider than its column was cut from the right, so the cursor and
  everything typed after the edge went out of sight. It now shows `…`, the
  last of the entry and the cursor.
- **The key map shows `priority_previous` whole.** Its action column was
  two cells narrower than that name, which read `priority_previ…` — the one
  column whose job is to be copied into a key table.
- **The watch log's "since you were here" line fits a narrow panel.** Below
  21 columns its label was drawn whole and cut by the terminal; it now ends
  in `…`.
- **Headlines keep their last letters once a story is selected.** The list
  moves every line two cells right to make room for its `▸` as soon as a
  story is selected, and the headlines were still wrapped to the whole
  width, so from the first `j`, `o`, `y` or `Enter` any row the wrap filled
  lost its last two letters with no `…` — `Krakatau` drawn as `Kraka`. They
  are now wrapped to the width they are drawn at. A source and age too long
  for the panel drop the age, and then cut the name with `…`, instead of
  being cut mid-word. A headline taller than the panel, shown clipped, now
  ends its last row in `…` instead of stopping wherever the wrap left it.
- **The news panel says `No stories.` when there are none.** A pass that read
  every feed and found nothing, or had no feeds to read, was never recorded,
  so the panel said `Reading…` for as long as it ran.
- **An Atom feed is reported instead of looking empty.** mirador reads RSS
  2.0, and the README said Atom too. An Atom feed, or an address that is not
  a feed at all, came back as an empty feed with no error, so the panel
  could not tell it from a quiet one. It now counts as a feed that failed,
  and a panel with nothing to show names it under `Cannot read the feeds`
  with the reason: "an Atom feed; mirador reads RSS 2.0 only", or "not an
  RSS feed". An RSS feed that simply has nothing in it today is still a
  quiet feed. The README now says RSS 2.0.
- **A feed with one fault in it keeps the headlines before it.** A single
  bare `&` anywhere in a feed, or one broken tag, threw away every headline
  in that feed. A bare `&` is now read as the ampersand it is, and any other
  fault keeps the headlines read before it while the feed is still reported
  as failing.
- **A bare `&` with a `;` after it no longer deletes the text between.**
  `R&D spending rises; analysts wary` was shown as `R analysts wary`, and a
  link whose address had both pointed somewhere else. A reference mirador
  cannot resolve — anything but the five XML entities, `nbsp` and a
  character number — is now shown as written, `&hellip;` included, which
  used to be dropped.
- **A rate-limited watchlist no longer asks for a source that does not
  exist.** The HTTP 429 message ended by saying another `[stocks].source`
  was needed, and `yahoo` is the only one there is: any other value stops
  startup. It now says, first, that Yahoo is rate-limiting this address,
  that the panel will try again, and that a datacenter or VPN address is
  refused outright.
- **The quote service's error text is bounded.** Yahoo's reason for
  refusing a symbol was kept whole and formatted into the markets panel on
  every frame, however long it was. It is now cut at 80 columns with `…`,
  twice anything real.
- **A temperature sensor whose chip name ends in a number keeps it.** A
  Linux sensor with no label of its own is named after its driver, and the
  panel took every digit off the end in turn, so `k10temp` on older AMD
  processors, `lm75` and `jc42` on many boards, and `it8728` came out as
  `k`, `lm`, `jc` and `it` in the table, the readout and the status-bar
  alert. Only the sensor's own number comes off now, and a number a
  separator introduces, like the `_1` in the kernel's `iwlwifi_1`.
- **Two tasks or notes sharing an id in a hand-edited file stay two.**
  Copying a `[[task]]` or `[[note]]` block is the natural way to add one by
  hand, and the copy kept its `id`. Every key acts by id, so deleting either
  deleted both, and editing the copy rewrote the original, with nothing said
  either time. A repeated id is now given a new one when the file is read,
  and the next save writes it down.
- **The task filter ignores case beyond `A` to `Z`.** It folded only those,
  so `übung` did not find a task called `Übung buchen`, though the notes
  search found the same words in a note. Both fold case the same way now, a
  letter at a time, which mends the notes search for Greek typed in
  capitals too: a capital sigma ending what had been typed folded to `ς`,
  so a note called `ΚΩΣΤΑΣ` dropped out of the search at `ΚΩΣ` and came
  back at `ΚΩΣΤ`.
- **The calculator takes `×` and `÷` to the full length of an entry.** The
  limit was counted in bytes where the panel counts characters, and those
  two signs are two bytes each, so a long entry using them was accepted as
  typed and then answered `too long`.
- **A clock named with no zone is refused for that.** `Tokyo =` in the
  clocks panel's add or edit dialog, a label with nothing after the `=`,
  was answered "that clock is already on the panel". It now asks for a zone
  after the `=`.
- **Choosing another calendar with `f` no longer fills the watch log.** The
  new calendar's first read was compared with the old one's events, so
  every event in it was logged as having "appeared in your calendar". It is
  now as quiet as the first read at startup, including when a read of the
  old file was already under way, and an event added to it later is still
  reported.
- **The `w` dialog scrolls on a short terminal.** It drew every widget mirador
  has with no window over them, so a terminal shorter than the list cut the
  bottom rows, footer first, while the cursor went on moving into them: `End`
  then `space` switched a panel on or off with nothing on screen saying which.
  The list now scrolls to keep the cursor in view, and the status and footer
  under it stay drawn down to a five-row terminal, the blank line above them
  giving way first. With room for everything, every widget is drawn at once,
  as before.
- **The `t` dialog's footer names the key you set.** It drew the keep key in
  capitals, so `keep = "y"` under `[theme_picker.keys]` was shown as `Y`,
  which is a different key and does nothing. Keys are now spelled as you
  wrote them, and both pickers spell `Esc` the way the key map does.
- **A plugin gets its 300 ms to clean up when it is closed.** Mirador tells a
  plugin to shut down and allows it 300 ms before ending it, but anything the
  plugin sent in that time — most often the frame it was already drawing —
  was treated as a protocol error and the process was killed at once. A
  plugin that saves its state on the way out could be stopped halfway through
  the save. A well-formed message sent during the grace is now ignored; a
  malformed one still ends the process at once.
- **A failed plugin shows the end of its error output.** The protocol
  promises the last few lines of a plugin's stderr in its panel, and the panel
  showed the first three — so a plugin that logged its startup and then
  crashed showed the startup, never the line saying what went wrong. It now
  shows the last three, giving up the earliest first when the panel is short,
  and a line the panel's height cuts ends in `…` rather than looking whole.
- **A pomodoro phase counts through sleep.** The timer kept its deadline on a
  clock that stops while the machine sleeps on macOS and Linux, so a focus
  interval with ten minutes left when the lid closed still had ten minutes
  left an hour later, and no chime had sounded. It now
  reads the wall clock as well and ends by whichever says less is left: the
  phase that ran out ends on waking, once, and the next waits at its full
  length for a key — or with `auto_start` begins from the moment you come
  back — rather than the timer racing through the phases you were away for. A
  wall clock set back can neither end a phase nor lengthen it, and a paused
  timer is untouched.
- **A pomodoro phase set longer than 180 minutes keeps its length.** `+` and
  `-` stop at 180 minutes, and the first press of either cut a longer phase
  from `[pomodoro]` down to it, took the difference off the time left —
  ending a phase with less than that to go — and remembered the 180 over the
  config. A longer phase now keeps its length: `-` takes a minute off, `+`
  puts it back as far as the config's length and no further, and a shortened
  length survives a restart.
- **The pomodoro's phase label says when it has been cut.** In a panel
  narrower than the label, `SHORT BREAK` was cut by the terminal to a
  whole-looking `SHORT BRE`. It now ends in `…`, and so does the time where
  it falls back to plain text.

## [1.19.2] - 2026-10-07

### Fixed

- **A calendar with an impossible time no longer crashes the agenda.** An
  hour of 25, a minute of 60, or a sign where a digit belongs in a
  `DTSTART`, `DTEND`, `EXDATE` or `UNTIL` made the time library panic on the
  agenda's reader thread, which left the terminal half-restored under a
  dashboard still drawing and the agenda never reading the file again. Such
  an event is now skipped and counted with the others the agenda could not
  read; a broken `EXDATE` is ignored and a broken `UNTIL` ends the rule at
  the date it carries. `T240000`, which ISO 8601 allows for the end of a day
  and some exporters write, is read as the midnight that begins the next.
- **A monthly event on the 29th, 30th or 31st stays on its day.** The rule
  was stepped from each occurrence to the next, and a month without that day
  is clamped to its last, so 31 January became 28 February and then the 28th
  of every month after. Each occurrence is now measured from the first, and a
  month that lacks the day is skipped and not counted, as RFC 5545 says. A
  yearly event on 29 February appears in leap years only.
- **A repeating event repeats in its own time zone.** Rules were expanded in
  the reader's zone, so a meeting on Mondays at 00:30 in London
  (`BYDAY=MO`) appeared in New York on Monday evenings, a day late every
  week. Each occurrence is now worked out in the zone the event was written
  in, then shown in yours.
- **A due date too far away is refused instead of crashing the dashboard.**
  Typing an offset past what the time library can hold into a task's due
  field — `99999y`, `999999m`, `9999999d` or `2000000w` — panicked and took
  everything down with it. It now gets the form's "out of range" line, the
  same answer as `9000y`, which never panicked but lands past the year 9999,
  and as a figure too long to read at all.
- **The agenda scrolls.** Its scroll keys and the mouse wheel moved a
  position the list was never drawn with, so the panel always showed its
  first rows and every event below the bottom edge was out of reach, while
  the border, `?` and the README all said the keys scrolled. They move the
  view now, a row at a time, ten with Page Up and Page Down, and to either
  end with `g` and `G`, and wherever the panel has room the day of the
  events on screen stays at the top as they scroll. A new day or another
  calendar starts at the top again.
  The panel also stops building a line for every event in the window on
  every frame, and builds only the ones on screen.
- **`SECURITY.md` describes the program that ships.** It still called
  mirador pre-1.0 with 0.7.x supported, said it contacts exactly two hosts
  when the default dashboard also reads three news feeds, and left feeds,
  calendars and plugin output out of the inputs worth reporting on. It now
  lists every host by panel, every file read and written, every program it
  can start, and puts a hostile feed, `.ics` file or plugin message in
  scope.

### Security

- **A news link is opened only if it is a web address, and never by a
  shell.** The README's Windows example for `[news].open_command` was
  `["cmd", "/c", "start"]`, and cmd reads its arguments as a command line:
  a feed whose link held `&` and a command got that command run when you
  pressed Enter on the headline, and an ordinary link with a query string
  opened cut short. mirador now refuses to hand a link to `cmd`, PowerShell
  or `pwsh` and says so in the panel, and the Windows example is
  `["rundll32", "url.dll,FileProtocolHandler"]`. If you copied the old line,
  replace it. Separately, Enter now opens only `http` and `https` links:
  `open` and `xdg-open` would otherwise hand a feed's `file:` or
  custom-scheme link to whatever program is registered for it. A link in
  another script is percent-encoded rather than refused, and one holding a
  control character is not opened.

## [1.19.1] - 2026-09-27

### Fixed

- **The battery panel reads on NetBSD** (#255). `starship-battery` 0.12.0
  carries the fix for its NetBSD backend, which passed the kernel's plist
  buffer with the trailing NUL still on it and so parsed nothing. Diagnosed
  and confirmed on hardware by [@0323pin](https://github.com/0323pin);
  fixed upstream in
  [starship/rust-battery#168](https://github.com/starship/rust-battery/pull/168).
  pkgsrc can drop its local patch.

### Changed

- **A battery holding below full is now reported by the system rather than
  inferred.** macOS says so directly once a charge limit is set, which
  `starship-battery` 0.12.0 surfaces as a state of its own; mirador had been
  reading it as "unknown, with no energy moving". The panel says `PLUGGED IN`
  either way, and the older inference stays for the platforms that still
  report it that way.

## [1.19.0] - 2026-09-24

### Added

- **Arrange mode's keys can be changed** (#284). An `[arrange.keys]` section
  moves any of them — the four moves, moving a whole row, keeping the
  arrangement, and picking another panel — and the legend shows the keys you
  chose. Esc still cancels and `1`–`9` still pick a panel. A key there may not
  also be a resize key, since resizing is read first. The key map on `?`
  pressed twice lists the table, and reload and reset include it.
- **The panel and theme pickers' keys can be changed** (#284).
  `[panel_picker.keys]` moves `w`'s keys (the cursor, `toggle`, `close`) and
  `[theme_picker.keys]` moves `t`'s (the cursor, `keep`, `put_back`), and each
  footer shows the key you chose. Esc always closes either picker. The theme
  picker's footer now draws Enter as `↵`, as the rest of mirador does.
- **The help overlay's scroll keys can be changed** (#284).
  `[help_overlay.keys]` moves `up`, `down`, `page_up`, `page_down`, `first`
  and `last`, and the overlay's footer shows the keys you chose. Any other key
  still closes it. None of them may be the help key, which opens the key map
  from there. With this, every key mirador reads comes from a table, except
  Ctrl+C, Esc and `1`–`9`.

### Fixed

- **The help overlay's footer lost its position when the overlay scrolled.**
  Since 1.17.0, a help overlay with more than fits (any panel with many keys
  on an 80x24 terminal) had a footer reading only `? key map ·`. The scroll
  position and the way to close it were dropped for lack of room, leaving a
  dangling separator. The position now comes first, the close hint is shorter
  while scrolling, and all three parts fit.

## [1.18.0] - 2026-09-24

### Added

- **Every panel's own keys can be changed** (#284). Each panel reads its
  keys from a table under its own section — `[todo.keys] delete = "x"`,
  `[calendar.keys] today = "."` — and its border, the status bar and the help
  overlay show the keys you chose. What moves is what a panel does while you
  look at it; forms, editors, search boxes and dialogs keep their keys, since
  they take typing, and `Esc` still backs out of each. The calculator has no
  table, its keys being what you type. The same rules as `[keys]` apply, and a
  panel key may be one the dashboard also uses, winning while that panel is
  focused, but not a resize key, which the panel would never see.
- **The watch log has a `[watchlog]` section**, holding only its keys.
- **The key map lists them**, each panel under the heading of the table its
  keys are written in. `r` reloads the panel tables along with `[keys]`,
  handing the new keys to panels already on screen, and `d` and
  `mirador --reset-keys` reset them too.

### Changed

- **`Enter` is drawn `↵` wherever a key is named**, as the panels' borders
  always drew it, and a config may write it either way.
- **Paired keys are drawn the same way in every panel**, following their
  actions' order: `↑ / ↓` and `k / j`, `n / p` for the calendar's months and
  `+ / -` for the pomodoro's length. Keys the panels read and the help
  overlay never mentioned — the calendar's `h`/`l`, the agenda's and watch
  log's `PageUp`/`PageDown`, `Home`/`End` in the watch log — are listed now.

### Fixed

- **The Mac advice for moving resize off Ctrl was wrong for Terminal.** It
  said letters with Alt work in every terminal and implied Option+arrows
  might. Measured on both: iTerm2 sends Option+arrows as Alt+arrows as they
  are; Terminal sends none intact, needs "Use Option as Meta key" for
  `alt+h`/`j`/`k`/`l`, and with it turns Option+↑/↓ into an `Esc` and two
  typed characters. The README and the shipped config now say so.
- The README said `PageUp` and `PageDown` move the task list a screen at a
  time; they move it ten rows, as the help overlay says.

## [1.17.0] - 2026-09-24

### Added

- **The dashboard's own keys can be changed** (#284). A `[keys]` section
  moves any of them — focus, help, quit, the panel and theme pickers, arrange
  mode and the four resize keys — to another key, to several, or to none, and
  the status bar, help overlay and arrange legend show the keys you chose.
  Written in words: `resize_wider = "alt+right"`. Ctrl+C and Esc cannot be
  bound, resize keys need Ctrl or Alt, and a key given to two actions is
  refused at startup. The panels' own keys are not configurable yet.
- **A key map, on `?` pressed twice.** Every key the dashboard reads, its
  default and what it does, with changed keys picked out and the config file
  to edit named. `r` reloads `[keys]` without a restart, reporting a mistake
  rather than applying it, and `d` puts every key back to its default after
  asking, by commenting out your `[keys]` lines.
- **`mirador --reset-keys`**, the same reset from the command line, for a
  keymap mistake that stops mirador starting. The startup error names it.

## [1.16.0] - 2026-09-18

### Added

- **The disk panel graphs I/O.** Under each device, a `↓ read ↑ write`
  readout and two braille histories in the network panel's face, scaled to
  the device's own peak and never below a megabyte a second, so background
  writes do not fill the graph. `i` hides them; `[disk].show_io`,
  `io_sample_secs` and `history` set the defaults. With the graphs on the
  panel scales to its row like the cpu panel.

## [1.15.0] - 2026-09-18

### Added

- **A disk panel.** Every volume that can fill up, as a line of figures —
  mount, percent used, free, capacity — over a meter, one block per device.
  Volumes that share a device are folded into one row, read-only ones are
  left out, and the reading happens on a thread so a sleeping network mount
  cannot stall the dashboard. Brass while there is room, amber at
  `[disk].warn_above_pct` (80), red at `[disk].alert_above_pct` (95), where
  the status bar names the fullest volume. Not in the default layout; `w`
  switches it on.

## [1.14.0] - 2026-09-18

### Changed

- **The battery panel draws a battery.** The charge was in block numerals over
  a meter, the pomodoro's face, and it read as a second clock. It is now a
  rounded cell with its terminal, filled to the charge, with the percentage
  in plain bold beside it. The cell grows with the panel and keeps its shape;
  a panel too short or too narrow for an outline falls back to a bare meter
  and the figure. The label, the detail row, the border's time left and the
  colours are unchanged.

## [1.13.1] - 2026-09-15

### Fixed

- **The clock's border offers `h`.** 1.13.0 put the 12-hour key only in the
  `?` overlay, so a clock with room to spare never showed it. It is now on the
  border and the status bar, after `Shift+↑↓ move` and before `d remove`, as
  `h 12/24h`. At the default width the border is unchanged; `h` appears as
  soon as the clock is wide enough for it.

## [1.13.0] - 2026-09-15

### Added

- **A 12-hour clock** (#265). `h` switches the clock panel between 24- and
  12-hour, and the choice is remembered; `[clocks].twelve_hour = true` sets
  it in the config. The default stays 24-hour. AM or PM sits small at the
  top right of the numerals, over the seconds, and the zone table follows:
  its `time_format` is converted to 12-hour with your padding kept. With
  12-hour off, `time_format` is used as written, as before.

### Fixed

- **A 12-hour zone row keeps its day marker.** A `time_format` with AM or PM
  in it, such as `%I:%M:%S %p`, was cut to `…` where the `+1d` belongs; the
  table now makes room for it.
- **A task's note preview says when it has been cut.** A note longer than
  the two-row preview lost its later rows with nothing to show it, so the
  seeded overdue task's note ended a sentence early and looked complete.
  The last visible row now ends in `…`.
- **The watch log's empty state reads as a sentence.** "which f on the
  agenda panel sets" was missing a word; it now says that pressing `f` on
  the agenda panel sets `[agenda].file`.

## [1.12.1] - 2026-09-14

### Fixed

- **The temperature panel's empty state says what is true on the platform.**
  The 1.12.0 hint told every Windows user to run as administrator; that is
  the answer on some machines and wrong on many, since Windows exposes at
  most one ACPI thermal zone and not every firmware provides one. On NetBSD
  it now says that `sysinfo` does not read envsys sensors yet (#255). The
  hint wraps to the panel rather than being cut.

### Changed

- **`ureq` 3.4.2**, which carries the upstream half of the #205 fix: a
  connect that fails for one resolved address now tries the next one
  instead of giving up (algesten/ureq#1195). mirador's own address-family
  fallback in `fetch.rs` stays, since a distribution may build against an
  older `ureq`. Also `dirs` 7, which changes nothing mirador reads.

### Security

- **`rustls` 0.23.45**, for RUSTSEC-2026-0285: 0.23.42 accepted TLS 1.3
  handshake messages sent at the wrong encryption level. The handshake is
  still authenticated, so a network attacker could not alter one, but a
  peer could send in plaintext what should have been encrypted. Every
  fetch mirador makes goes through it.

## [1.12.0] - 2026-09-11

### Added

- **A battery panel, for laptops.** The charge in block numerals with the
  meter beneath it, a label for which way things are going, the time left or
  to full in the border, and a line of detail — health, cycles, power draw.
  Amber below 20%
  and red below 10% only while running on the battery, and below 10% the
  status bar carries an alert. Not placed in the default layout, since a
  desktop would open on `No battery`; `w` switches it on. `[battery]` in the
  config sets the two thresholds and the sample interval.
- **A temperature panel.** The hottest sensor with its history in the CPU
  panel's face, and a table of every sensor group's current and peak
  reading. The platform's list is grouped the way a person would write it —
  Apple silicon's fourteen die readings become one `CPU die` row, and
  sensors reporting impossible values are dropped. `u` toggles Celsius and
  Fahrenheit, and the choice is remembered. Above `alert_above_c` the status
  bar says so. Also not in the default layout: on Windows the sensors need
  elevation, and a VM or a container has none, so the panel says so rather
  than drawing an empty graph.

## [1.11.0] - 2026-09-11

### Added

- **Three more bundled themes, completing two pairs.** `everforest` is the
  dark half of `everforest-light`; `rose-pine` and `rose-pine-moon` are the two
  dark modes of the palette `rose-pine-dawn` is the daylight of. Each mirrors
  its sibling role for role and cites the upstream values it took. Nineteen
  themes ship.

- **A memory panel.** Memory in use as a percentage, a moving braille chart
  of it, and — on a machine that has swap — a swap meter that `s` hides and
  shows. It is `cpu`'s sibling in every respect and sits beside it on the
  instrument row of the default layout, which now holds five panels; the
  markets grid keeps its width and its change column, and the timer and the
  two graphs each gave up a little. `[memory]` in the config carries the same
  `history` and `sample_secs` as `[cpu]`.

### Fixed

- **A readout squeezed below the width of its first value could lose its
  unit without saying so.** `38` for `38%`, at a width of three cells, in any
  panel whose first value is a figure beside a unit. The abridging now marks
  the cut with `…` wherever it falls. Found by the memory panel's own tests
  on the day it was written.

## [1.10.2] - 2026-09-10

### Fixed

- **The clock's small seconds could be cut to one digit.** When the panel is
  too narrow for the full block-numeral time, it draws `HH:MM` in numerals with
  the seconds small beside them — and the numerals were sized without counting
  the three cells the seconds take, so at some widths the terminal cut `26`
  down to `2`. The pair is now sized together, and where even that does not
  fit the clock steps straight down to plain text with every digit intact
  rather than to numerals with the seconds silently missing.

- **Four more places were cut by the terminal at narrow widths**, found by
  the same sweep: the clock's plain-text fallback, the notes' `written …
  edited …` line, the agenda's day headings and event rows, and the cpu
  panel's `PER CORE` label. Each now abridges with `…` or drops a whole value
  rather than leaving a fragment. The cpu panel's per-core strip, which drew
  one mark per column and stopped at the edge, ends in `…` when there are
  more cores than columns.

## [1.10.1] - 2026-09-09

### Changed

- **The notes panel no longer prints its count twice.** The border already
  carries it, so the row above the list is spent on an active search or
  nothing at all — and when it is nothing, the list and the note it is
  pointing at get the row. The count returns for the two cases where the
  border stops carrying it: an empty panel, where `no notes` is the only
  thing saying the panel is working, and a failed save, where the counter is
  spent on `unsaved!`.

## [1.10.0] - 2026-09-09

### Fixed

- **The clock's date was cut without saying so.** On a narrow terminal
  `WEDNESDAY 09 SEPTEMBER` came out as `WEDNESDAY 09 SEPT` — the line was
  handed to a paragraph narrower than itself, so the *terminal* did the
  cutting, and a terminal leaves no mark. It now ends in `…` like every other
  abridged line on the dashboard, and its width is measured in terminal cells
  rather than characters, so a `date_format` holding double-width text is
  placed correctly too.

### Changed

- **The resize keys are drawn rather than spelled: `Ctrl+←→↑↓`.** They are the
  same four arrows the arrange legend already draws two hints away, so the
  status bar, the arrange legend, the help overlay and `--help` now show the
  pair as one idea — plain arrows move a panel, the same arrows with `Ctrl`
  resize it.

- **The status bar fits in a narrower terminal.** With the drawn arrows and a
  two-space gap between hints, the whole bar appears from 83 columns where it
  previously needed 92. Hints still drop whole rather than being cut.

- **The tasks panel no longer prints its open count twice.** The border
  already carries `4 open`; the summary line now spends its width on what is
  wrong and how the list is sorted. The count returns to the summary in the
  one case where the border is saying `unsaved!` instead.

- **The markets panel shows one more symbol.** `via yahoo` has moved into the
  frame — `┤7 · yahoo├` — instead of holding an interior row open for ever in
  the panel that has the fewest of them. The row underneath returns whenever
  there is something to say there, which is where a failing fetch still
  explains itself.

- **The help overlay's key column is sized to the keys it shows** rather than
  to a fixed width, so its actions start beside the keys instead of a third of
  the way across the dialog.

- **A panel too narrow for its own name draws no title** instead of a lone
  `┤…├`, which spent three cells saying a title had been cut and nothing about
  which panel it was. A jump key keeps its place: `┤4├` is still an answer.

### Added

- **Cutting a release no longer needs the owner's machine.** Two dispatchable
  workflows cover the two manual steps: `cut-release.yml` tags the head of
  `main` with the manifest's version, starting the release build exactly as a
  manual tag push would, and `publish-crates.yml` publishes a tag to crates.io
  using Trusted Publishing, so no long-lived crates.io token exists anywhere.
  Both steps remain runnable by hand from a machine with the right
  credentials.

## [1.9.0] - 2026-09-01

### Added

- **Headlines are hyperlinks.** In a terminal that renders OSC 8 links —
  iTerm2, WezTerm, kitty, recent GNOME Terminal among them — a news headline
  is now clickable and opens the story the way the terminal opens any link
  (usually a modifier plus a click, since mirador holds the mouse). Nothing
  changes anywhere else: the sequences occupy no cells, so a terminal
  without support shows exactly the panel it always did, and `o`, `y` and
  `↵` are untouched. A feed's URL rides *inside* an escape sequence, so only
  plain `http(s)` URLs made of RFC 3986 characters are linked — anything
  else (a control byte smuggled through an entity, an off-web scheme) gets
  no link rather than an escaped one.

### Changed

- **`quick-xml` 0.41 → 0.42**, which rebuilt its API around `str` instead of
  byte slices. The feed parser is ported with identical behaviour — the
  entity corpus and trim-once tests pass unchanged, and live feeds were
  driven through it before merging. The dependency floor stays at Rust 1.95.

## [1.8.0] - 2026-08-25

### Added

- **Six more bundled themes, chosen light-first.** The first six ports were
  all dark, so this batch is five light and one dark: `solarized-light`,
  `gruvbox-light`, `catppuccin-latte`, `rose-pine-dawn`, `everforest-light`,
  and `kanagawa` (Wave). Each is a port in the established sense — hex values
  from the palette's own specification, cited in the file, with the mapping
  mirroring its dark sibling where one exists and body text left on the
  terminal's own foreground. Light themes start their graph ramps light, so
  an idle chart recedes into a pale background.

### Changed

- **NetBSD appears in the platform badge and the install table**, pointing at
  pkgsrc's `sysutils/mirador` — the community package that arrived with
  1.6.1's fix.

## [1.7.0] - 2026-08-25

### Added

- **The markets panel colours a row by the size of the day's move.** The
  change, percentage and sparkline now take their colour from the theme's
  `gain_gradient` / `loss_gradient` ramps, saturating at a 2% move — a
  drifting tenth of a percent sits at the ramp's dark desaturated foot
  instead of shouting the same green or red as a crash. Flat and stale rows
  stay muted, exactly as before. This is what those two theme keys were
  reserved for; themes that already set them light up unchanged.

### Changed

- **The shipped config now documents external panels**, so `--print-config`
  and a first run's `config.toml` finally show `[[plugins]]`, alongside newly
  documented clamps and floors and two reserved theme gradient keys. A
  documentation-review pass also corrected the protocol specification
  (undocumented bounds an SDK author could trip on, two wrong claims about
  cursor and mouse handling), the README (the config file's real name, the
  three persistence destinations, NetBSD via pkgsrc's `sysutils/mirador` in
  the install section) and CONTRIBUTING (the widget checklist's missing
  layout step, the shared fetch helper).
- **`ureq`'s declared floor is 3.4.0**, the version every release since 1.5.1
  has actually shipped; the 1.5.1 entry announced the bump but the manifest
  minimum was never raised.

## [1.6.1] - 2026-08-25

### Fixed

- **Fetching works again on machines that prefer IPv6 without an IPv6 route.**
  When DNS returns an AAAA record first and the kernel has no route to it,
  every connect died instantly with `No route to host` and the IPv4 address
  was never tried — a condition browsers and curl mask with happy-eyeballs
  fallback, so the machine looks healthy everywhere except mirador. A request
  that fails unroutable is now retried pinned to one address family at a
  time. Reported from NetBSD in
  [#205](https://github.com/jchultarsky/mirador/issues/205).

## [1.6.0] - 2026-08-24

### Added

- **External panels are an explicit, language-neutral process boundary.** A
  configured command can publish styled frame snapshots and opt into bounded
  key, paste and mouse events. Mirador embeds no interpreter,
  performs no plugin discovery, and starts no process unless its id is placed
  in the layout. Each process is isolated from the UI thread; startup, message
  rate, retained output and input queues are bounded; Mirador owns cell-aware
  wrapping and clipping; external tiles are labelled; and Ctrl+C remains an
  unconditional host-owned exit even when a child is stuck. The protocol is
  documented independently so SDKs do not link against Mirador's private Rust
  `Panel` trait. External panels can also hand bounded, completed events to
  Mirador's native Watch Log. Protocol v1 is a compatibility commitment from
  this release — see the README's Compatibility section.

## [1.5.1] - 2026-08-24

### Added

- **Linux aarch64 binaries.** Releases now carry `mirador-aarch64-unknown-linux-gnu.tar.gz`,
  built natively on GitHub's ARM runners, alongside the x86-64 Linux archive.

### Changed

- **`ureq` 3.3.0 → 3.4.0.** Pooled connections now age out as configured, and
  IPv6-literal hosts present the right TLS server name. No behaviour change on
  the paths mirador exercises; the fetch stack was driven live before release.

## [1.5.0] - 2026-08-02

Both features in this release came from outside the project, contributed by
[@krflol](https://github.com/krflol).

### Added

- **Notes can select, copy and paste body text as a scratchpad.** Shift plus
  the navigation keys extend a visible selection and `Ctrl+A` selects the
  whole body. `Ctrl+C` sends the selection to the terminal clipboard while
  retaining an internal copy, and `Ctrl+V` inserts or replaces from that copy
  even when the terminal declines OSC 52. Typing and deletion replace selected
  text as expected, bracketed terminal paste preserves outside multiline text,
  and the editing border and footer expose the active keys.
- **`mirador --update` upgrades both installer- and Cargo-managed copies.** It
  hands release installs to the existing `mirador-update` helper and crates.io
  installs back to Cargo, so the dashboard can advertise one command without
  knowing how it was installed. On Windows it moves the running executable
  aside before waiting for the updater, because Cargo cannot replace a mapped
  `.exe`; the next launch removes that old image. The standalone updater stays
  available for compatibility, and no network request happens without an
  explicit update command or the existing opt-in update check.

## [1.4.1] - 2026-08-01

### Fixed

- **A long note or task note no longer slows the whole dashboard down.** The
  notes reader wrapped a note's entire body on every frame and then scrolled
  past most of it, and the task panel did the same to fill a two-row preview —
  so the cost of drawing was proportional to what you had written rather than
  to what was on screen. A two-megabyte note cost 62ms a frame against a 250ms
  tick; it now costs under a third of a millisecond, and the cost no longer
  grows with the note.

  Nothing about what is drawn has changed.

## [1.4.0] - 2026-08-01

### Added

- **Calculator results can be selected, copied and reused from the tape.**
  `↑`/`↓` moves the full-weight selection through earlier answers, `y` sends
  that answer to the terminal clipboard, and `p` pastes it into the live
  expression. Once a result exists, the focused panel's compact border hint
  advertises `y copy · p paste` at the default width.

  Contributed by [@krflol](https://github.com/krflol) in
  [#174](https://github.com/jchultarsky/mirador/pull/174) — the first change to
  mirador from outside.

### Fixed

- Two tests spawned `true`, which does not exist on Windows, so the suite failed
  for anyone building there. It passed in CI because GitHub's Windows runners
  ship Git for Windows and it puts a `true.exe` on `PATH` — a green CI is
  evidence about the runner as much as about the code. Found by @krflol running
  the suite on their own machine.

## [1.3.2] - 2026-08-01

### Fixed

- **The README's demo recording and its captions.** Both captions said "All
  twelve panels"; the default layout has placed thirteen since the calculator
  arrived, and the recording predated it. The GIF is referenced by absolute URL
  because `/docs` is excluded from the crate, so replacing it updates the image
  on every published version at once while the prose around it stays frozen at
  whatever each release baked in — which is why the corrected captions need a
  release of their own rather than riding along later.

  The recording now shows the calculator, typed a character at a time so the
  answer can be seen forming before Enter, with three sums chosen to demonstrate
  the tape's decimal alignment.

## [1.3.1] - 2026-08-01

### Changed

- **The calculator is drawn as an adding machine's tape** rather than as one
  large number. The version that shipped in 1.3.0 put the answer in block
  numerals, which was reasoned from a resemblance — the clock and the pomodoro
  use them — rather than from what a calculator is. Those two show a single
  continuously changing value you glance at, and the numerals exist so it reads
  across a room; a calculated answer is read once and checked against the working
  that produced it. The numerals also cost six cells a digit, so they only ever
  appeared for answers small enough not to need them.

  The panel now has `WORKING` and `RESULT` columns like the other lists here,
  oldest entry at the top and the line you are typing at the foot, where a tape
  feeds from. Results are aligned on the decimal point. The answer forms beside
  the line as you type, and Enter keeps it.

  The row you are typing is the brightest thing in the panel with its answer in
  brass; entries behind it recede.

### Added

- **A clear key for the calculator.** `c` clears what you are typing and `C`
  clears the tape as well — the CE and AC of a desk calculator. Previously only
  `Esc` did this, listed as a secondary binding, so it was easy to miss.

### Fixed

- The calculator could show a truncated answer in two narrow-panel cases: a
  result handed to a column narrower than itself, and a result pushed back over
  the edge by the padding that aligns decimal points. A number missing its tail
  is a different number, so both now degrade to scientific notation or give up a
  place of alignment instead.
- Calculator error phrases were too long for the column they are drawn in, so
  `cannot divide by zero` appeared as `cannot divid…`.

## [1.3.0] - 2026-08-01

### Added

- **A calculator panel.** Type an expression, press Enter. Suggested by a reader
  on r/rust.

  `2 + 3 * 4` is 14, not 20 — precedence is the arithmetic kind and brackets
  work. A pocket calculator gives 20, and that was the other candidate; it lost
  because what this replaces is `bc` or `python3 -c`, not a desk calculator, and
  anyone typing `2+3*4` at a terminal would read 20 as a bug. An operator typed
  straight after an answer carries it forward, which covers what a memory key
  was for. `y` sends the answer to the clipboard, and what you have worked out
  lands on a tape below.

  An answer too wide for the panel is shown in scientific notation rather than
  cut. Everywhere else in mirador a value that will not fit reads as a narrow
  terminal; here a number missing its tail is a different number, and nothing on
  screen would say so.

  **While this panel has focus, `1`–`9` type digits instead of jumping to a
  panel.** It is the only panel that changes what a global key does, and it is
  unavoidable — a calculator needs the digits. `Tab` still moves focus, and `q`,
  `?`, `w`, `m` and `t` all still work.

  There is no memory key, no percent key and no functions, and nothing is kept
  when you quit.

### Changed

- The default layout now places thirteen panels rather than twelve, with the
  calculator on the reading row beside the news and the watch log. Jump keys
  stop at `9`, so the pomodoro joins the CPU and network panels in not having
  one. An existing config is never rewritten, so this affects new installs and
  configs with no `[layout]` block.

## [1.2.0] - 2026-08-01

### Fixed

- **Panels no longer let the terminal cut a value in half.** A line built at its
  natural width and handed to the renderer is not truncated by mirador — it is
  truncated by the terminal, which keeps the cells that fit and drops the rest
  without saying so. The terminal cannot tell a value from a fragment, so it cuts
  wherever the edge falls, and a fragment of a value is not a smaller truth.

  Found by rendering every panel across a range of widths rather than by reading
  the code. Four panels were cutting values at ordinary terminal sizes:

  - the **network** readout showed a bare `↑` with no upload figure beside it at
    100 columns, and `6.4 KB` — a total — where `6.4 KB/s` was meant;
  - **weather** showed `humidity` with its percentage gone;
  - the **pomodoro** footer trailed off as `25m focus ·` at the shipped default
    of 120 columns;
  - the **calendar** cut a date down the middle, so `14` under THU became
    Thursday the 1st, with nothing on screen to say otherwise.

  Values are now dropped whole — a figure leaves with its unit, an arrow with the
  number it points at — and prose is ellipsised, so an abridged message says that
  it is. Where a reading would be dropped only because of the padding that keeps
  it from jiggling, the padding goes first. The calendar drops whole weekday
  columns instead of half a date.

- **The empty-state messages in the agenda and watch log lost a word** when the
  panel was narrow. Both were hand-broken into two lines that still read as a
  whole sentence — `Nothing has` above `since 00:30.` — so the omission was
  invisible. They wrap now.

- **A table could build a row wider than the panel it was resolved for.** Column
  widths are declared without reference to the total, so a pane narrower than
  their sum overflowed and the last value on the row lost its tail. Long-standing
  and only reachable at small sizes, but the same defect as the four above.

- The status bar and the arrange-mode legend were doing this arithmetic by hand,
  and one of them measured bytes rather than display cells.

## [1.1.3] - 2026-08-01

### Fixed

- **The agenda panel's "no calendar yet" message was cutting off the path.** That
  message exists to tell you where to put your `.ics`, and it stopped mid-path —
  `Looked in /var/folders/zj/blsvny` — which is not an instruction. It wraps now,
  as does the prose above it, which had been hand-wrapped for one panel width and
  clipped at every narrower one, and as does the failure reason when a calendar
  cannot be read.

  Same shape as the markets fix in 1.1.2: the terminal clips whatever does not
  fit, so anything built without asking how wide the panel is will eventually say
  something untrue or unreadable.

## [1.1.2] - 2026-07-31

### Fixed

- **The markets panel was showing wrong numbers in the default layout.** Its row
  was built at full width whatever the panel could show, and the terminal
  clipped whatever hung over the edge — so a change of `+52.07` was drawn as
  `+52.0`. Every column now has a width below which it is dropped instead, in
  order of expendability: the sparkline first, then the percentage, then the
  change, then the last price. A missing column reads as a narrow terminal; half
  a number reads as a different number.

  Even the symbol has a floor, because a ticker clipped to `BRK.` is as wrong as
  a price clipped to `+52.0`.

- **The markets panel gains two columns in the shipped layout**, taken from the
  cpu graph, which scales to whatever it is given. Without them the fix above
  would have cost the default dashboard its change column — and the change is
  the answer to "what is the portfolio doing", which is one of the four
  questions this dashboard exists to answer. Existing configs are untouched; a
  config is seeded once and never rewritten.

## [1.1.1] - 2026-07-31

Working notes and test coverage. **Nothing you can see changes** — the binary
behaves identically, and the README and shipped config are untouched. Published
so the source on crates.io matches the repository.

### Changed

- The working notes now record what shaped the two reset flags: why they are
  separate commands rather than degrees of one, why resetting the config has to
  clear the remembered preferences with it, and why a factory reset decides what
  it may touch by which program wrote the file rather than by where it sits.
- A test now checks those notes for the kinds of staleness a machine can see — a
  test cited by name that no longer exists, a released version disagreeing with
  the manifest, a cited path that has moved. It found one on its first run. Most
  of that file is prose about the world and stays unchecked, which the test says
  out loud rather than implying otherwise.

## [1.1.0] - 2026-07-31

### Added

- **`mirador --factory-reset` puts everything back to how it arrived.** The
  reset `--reset-config` could never honestly be: it resets configuration, while
  your watchlist, tasks and notes outlive it. This sets aside every file mirador
  has written — config, remembered preferences, tasks, notes, watchlist and
  world clocks — and the next launch seeds them all again, default stock tickers
  included. A reset install is now indistinguishable from a new one.

  **Nothing is deleted.** Every file is renamed to a `.bak` beside itself, so it
  is something you can walk back from with `mv`, and an existing backup is never
  overwritten. That is also why a plain `y` is enough to confirm it: the prompt
  lists every affected file by full path, and the worst outcome is renaming
  things back rather than lost work.

  Your calendar is not touched. mirador only ever *reads* an `.ics`, so that
  file is yours even when it sits in mirador's own directory — and neither are
  files at paths you chose yourself with `[todo].file` and its siblings.

## [1.0.4] - 2026-07-31

### Fixed

- **`--reset-config` now resets what you can see.** It restored the config
  correctly all along, but left `state.toml` — the preferences you change from
  the keyboard — in place. Those *outrank* the config, so they were applied
  straight back over the file just restored, and the dashboard came back looking
  exactly as before. Anyone who had only ever changed their theme saw the
  command appear to do nothing. The remembered preferences are now put aside
  too, into `state.toml.bak`, and a second reset does not overwrite the first
  copy.

  Both the prompt and the result say what is kept, because the boundary was
  invisible from the flag's name: **your tasks, notes and watchlist are left
  alone.** They are your content rather than configuration, so a command called
  `--reset-config` does not delete them — which also means the default stock
  tickers are not restored. `[stocks].symbols` seeds `watchlist.toml` only when
  that file is absent, and that is what lets the panel edit the list at all.

## [1.0.3] - 2026-07-31

Documentation only. No behaviour changes at all; this exists so the corrected
text reaches the README on crates.io and the config mirador prints.

### Changed

- **The resize key is spelled the same everywhere.** The status bar, arrange
  mode and `--help` all say `Ctrl+arrows`; the README said `Ctrl+arrow` in four
  places, including both key tables, so the page you read and the bar you look
  at disagreed about one key. The comment at the head of the shipped config said
  it too, and that text is compiled into the binary, so `mirador --print-config`
  carried it. The singular survives where it means one keypress — that is
  grammar, not inconsistency.

## [1.0.2] - 2026-07-31

Housekeeping. Nothing you can see changes; both entries are things that were
wrong underneath.

### Fixed

- **A very long watchlist collapsed the stocks panel instead of filling it.**
  The panel reports the height it can use, and that sum was not saturating while
  already reaching for `u16::MAX` — so a watchlist of around 65,000 symbols
  wrapped it to three rows. Absurd in practice; the watchlist is a file you
  edit, so nothing bounded it. The cap itself was checked at the same time and
  is exactly right: the panel is complete at header, every symbol and the status
  line, and any extra height would be a blank gap, so the space goes to a
  neighbouring row instead.

### Changed

- **The test suite no longer reaches the network.** It was documented as never
  doing so, and that had quietly stopped being true: constructing the stocks
  panel spawns a fetch thread, so *building* one called Yahoo Finance — and
  since the default layout places that panel, any test building a dashboard did
  too. Quote sources are injectable now, and the one function that opens a
  socket refuses outright under `cfg(test)`. No effect on the shipped binary,
  which fetches exactly as before.

## [1.0.1] - 2026-07-31

### Fixed

- **The resize keys are advertised where you would look for them.** `Ctrl+arrow`
  has resized the focused
  panel since long before 1.0, but it was declared as a secondary binding, so it
  appeared only in the `?` overlay — someone looking for it on the status bar
  concluded the feature did not exist. It now sits beside the other primary
  hints, spelled `Ctrl+arrows resize`, the way arrange mode and `--help` already
  spelled it. A panel that cannot use the extra space still declines it, so
  growing a row may move the space to a neighbour rather than to the panel you
  are pointing at; that is deliberate.
- **The status bar no longer cuts a hint in half.** It drew every hint and let
  the terminal clip the last one, which on a narrow window left fragments like
  `Ctrl+←` — a rendering fault to look at, where a missing hint just reads as a
  narrow terminal. It now drops whole hints, which is what arrange mode's legend
  has always done. Reachable before this release for any terminal narrow enough
  to cut `t theme`.

## [1.0.0] - 2026-07-30

**1.0 is a promise, not a feature.** Nothing is added here that was not in
0.19.0. What changes is what the version number commits to:

- **Your config keeps working.** No option that has ever shipped has been
  removed. Re-verified before tagging against all thirty-one release tags —
  1860 key comparisons, nothing lost. The four keys predating `0.1.0` are still
  handled by `mirador --migrate-config`.
- **Your data files keep working.** `todos.toml`, your notes, `watchlist.toml`,
  `zones.toml` and `state.toml` have not changed shape since `0.1.0`, and all
  ignore keys they do not recognise — so a file written by a newer mirador still
  opens in an older one.
- **No known crashes or hangs.** Every module has been read adversarially, the
  untrusted-input boundary is bounded on every side, and the dashboard has been
  soaked across real midnights on macOS, Linux and Windows.

Breaking changes from here get a major version. Options may be added; they will
not be renamed out from under you.

## [0.19.0] - 2026-07-30

### Added

- **A story's link can now be copied or opened, not just looked at.** `y` asks
  the terminal to put it on the clipboard (OSC 52) — no configuration and no
  dependency, though some terminals refuse it and tmux needs `set-clipboard on`;
  mirador cannot tell either way, so it says it *sent* the link rather than
  claiming it copied one. `↵` opens it with `[news].open_command`, which is empty
  by default — mirador launches nothing you did not name, runs it directly rather
  than through a shell, and passes the link as its own argument so nothing in a
  URL can be read as shell syntax.
  [#137](https://github.com/jchultarsky/mirador/issues/137).

## [0.18.2] - 2026-07-30

### Fixed

- **The watch log erased its own "since you were here" line the moment you came
  back.** It marked the log seen on gaining focus as well as losing it, so
  returning to the dashboard set "last looked" to *now* — everything that
  arrived while you were away landed on the old side of the line, and the line
  disappeared in the instant you came back to read it. A terminal that reported
  focus *correctly* made the feature less visible than one that did not. Only
  losing focus marks it seen now.
  [#132](https://github.com/jchultarsky/mirador/issues/132).

- **The clock's reorder keys were invisible.** `Shift+↑`/`↓` shipped in 0.17.0 as
  an `extra` binding, which put it in the help overlay and nowhere else — so the
  panel border advertised `e edit` and said nothing about reordering, the thing
  [#109](https://github.com/jchultarsky/mirador/issues/109) was actually asked
  for. It is now a primary and sits above `d remove` on the border. `a add zone`
  is shortened to `a add` to make room; `d` still deletes and still appears on a
  wider panel, and it is the key every other list panel already teaches.

## [0.18.1] - 2026-07-30

### Changed

- **A story's link no longer hides in plain sight.** `o` drew it in the same
  verdigris every masthead wears, so it was camouflaged by repetition and
  appeared without drawing the eye. It is now brass — the dashboard's attention
  colour — and prefixed `↳`, with wrapped lines aligned under it.
  [#117](https://github.com/jchultarsky/mirador/issues/117).

### Fixed

- **`o` did nothing until you moved the cursor.** The selection starts empty, so
  on a freshly focused news panel the key was silently ignored while the border
  advertised `o show link` regardless. It now shows the top story's link and
  marks the story it came from.

## [0.18.0] - 2026-07-30

### Added

- **Arrange mode can move a row, not just a panel.** `Shift+↑`/`↓` (or `J`/`K`)
  moves the whole row the focused panel sits in. Before this, a new row was only
  ever created by pushing a panel off the top or bottom edge, so a panel alone in
  a *middle* row could not travel at all — `Down` merged it into its neighbour
  and the row count fell. Going from `[clocks] [watchlog] [notes] [cpu]` to
  `[clocks] [notes] [watchlog] [cpu]` was not expressible with any sequence of
  keys. Moving a panel between rows still merges, exactly as before.
  [#100](https://github.com/jchultarsky/mirador/issues/100).

## [0.17.1] - 2026-07-30

### Fixed

- **The news panel scrolled, which its own rule said it must not.** Every story
  became a list item, so the selection could walk past the bottom of the panel
  and ratatui's `List` scrolled to follow it — with the shipped feeds that made
  nine of twelve stories reachable only by scrolling. "However many stories fit,
  and no more" was documentation rather than behaviour, and nothing tested it.
  The panel now builds only the stories whose whole block fits, so the cursor
  cannot leave the viewport and there is nothing to scroll. A taller panel still
  shows more. [#118](https://github.com/jchultarsky/mirador/issues/118).

## [0.17.0] - 2026-07-30

### Added

- **The clock's zone list can be reordered, edited and located.** `Shift+↑`/`↓`
  (or `J`/`K`) moves the selected clock through the table, `e` opens it in the
  same `Label = Zone` dialog `a` uses, pre-filled, and `o` shows where
  `zones.toml` lives. Previously the only way to change an order or fix a label
  was to delete entries and re-add them in the order you wanted, which is what
  the reporter did. The first entry is still the big clock and still cannot be
  displaced — reordering the table is what was asked for; choosing the primary
  is a different decision and would want its own key.
  [#109](https://github.com/jchultarsky/mirador/issues/109).
- **`--reset-config`, a way out of a config that has gone past fixing.** Writes
  the shipped defaults and copies the old file to `config.toml.bak` first. The
  name sounds harmless and the effect is not, so it says what it is about to do
  and waits for a `y`; piped somewhere with no terminal to ask on, it refuses
  rather than assuming, and `--yes` is there for scripts that mean it. An
  existing backup is never overwritten — resetting twice is exactly what a stuck
  reader does, and with a fixed name the second run would have replaced the real
  config with the defaults written by the first.
  [#111](https://github.com/jchultarsky/mirador/issues/111).

### Fixed

- **The agenda kept saying `reloading…` after the reload had finished.** The
  status was cleared only by the next keypress, so a panel nobody touched went
  on claiming to be mid-operation — measured at 83 seconds with the reloaded
  events already on screen. A dashboard is read without being touched, so
  "cleared on the next keypress" was, for anyone glancing at it, never.
  [#120](https://github.com/jchultarsky/mirador/issues/120).
- **The watch log told you to set a calendar you had already set.** It read
  `[agenda].file` once at construction, so a calendar added later with `f` was
  never noticed and the panel went on advertising `f` until a restart. It now
  says where calendar entries come from and asserts nothing about whether you
  have one. [#119](https://github.com/jchultarsky/mirador/issues/119).

## [0.16.3] - 2026-07-30

### Fixed

- **The news panel did not show which story the cursor was on.** It kept a
  selection that `j`/`k` moved and that `o` read the link from, and drew no
  highlight at all — so the link at the foot of the panel belonged to a story
  you had no way to identify. The task, notes and watchlist panels have always
  marked their selection; this one never did.
  [#114](https://github.com/jchultarsky/mirador/issues/114).

## [0.16.2] - 2026-07-30

The first round of bug reports from people who are not the author, and one
defect found while confirming one of them.

### Fixed

- **Hiding the seconds left them showing in the zone table.** `s` is bound to
  "seconds" and the clock is one panel, but the secondary zone list formatted
  from `[clocks].time_format` and never consulted the setting — so `s` gave you
  a clock with no seconds directly above a table that still had them. Your own
  format survives it: the seconds specifier is removed from *your* format rather
  than swapped for a fixed one, so `%I:%M:%S %p` stays a 12-hour clock. Reported
  by email; [#106](https://github.com/jchultarsky/mirador/issues/106).

- **The `+1d` / `-1d` day marker was silently truncated out of the zone table.**
  The column held the time *and* the marker in nine cells, and `02:43:48 +1d` is
  twelve — so the marker was cut every time a zone was on a different date,
  which is every time it matters. It is the half of that row that carries the
  warning, and the reason it exists is that a day boundary is the thing people
  get wrong. Found while confirming the above, not reported;
  [#107](https://github.com/jchultarsky/mirador/issues/107).

- **Pressing `o` on a news story cut the URL off.** It was truncated to the panel
  width, so the link could not be read or copied — and the terminal linkified the
  visible text, which now ended in an ellipsis, so clicking it went to a URL that
  does not exist. The footer now takes the rows the whole link needs. Reported by
  email; [#108](https://github.com/jchultarsky/mirador/issues/108).

## [0.16.1] - 2026-07-29

A hotfix for one reported bug, and a second one of the same shape found beside
it.

### Fixed

- **Hiding the seconds on the clock made the time render in small text.**
  Pressing `s` to drop the seconds should make the numerals larger if anything;
  instead the clock could fall back to plain text entirely.

  The scale search took only a width, and the callers filtered its answer by
  height afterwards — which rejects rather than stepping down a size. Because a
  shorter string fits a *bigger* scale, and each scale is five rows taller than
  the last, `HH:MM` could earn a scale that was wide enough but too tall, and
  lose its block numerals altogether. Reproduced at ordinary sizes: any terminal
  around 74-110 columns by 15-17 rows with the clock panel given the width.
  **Present since 0.1.0.** Reported by @abusch in
  [#103](https://github.com/jchultarsky/mirador/issues/103).

- **The pomodoro timer could lose its numerals partway through a session**, for
  the same reason and in the same helper. A focus period over 99 minutes renders
  `180:00` and counts down to `99:59`, and the shorter string could earn a scale
  too tall to draw. Found while fixing the clock rather than reported.

## [0.16.0] - 2026-07-28

Four adversarial review passes, completing the first phase of the work towards
1.0. Two of them found crashes, one found a way for a third-party news feed to
grind the dashboard down, and one found a bug in code three days old.

### Removed

- **The unused-widget notice.** mirador used to name the widgets your layout
  does not place — once in the status bar at startup, and permanently in the
  help overlay. The status bar line retired on your first keypress; the overlay
  section did not, so if you had deliberately switched four panels off you were
  told about them every time you pressed `?`.

  A dashboard cannot tell "has not discovered this yet" from "decided against
  it", so a hint aimed at the first reminds the second for ever. `w` is still on
  the status bar and in the help, so the way to switch a panel on is unchanged;
  what is gone is being told that you should.

### Changed

- **The default watchlist leads with the major US indexes** — S&P 500, Dow,
  Nasdaq Composite, Russell 2000 and Nasdaq-100 — keeping `AAPL` and `MSFT` so
  the panel shows both kinds on a first run. This seeds the watchlist file on a
  *first run only*, so an existing installation is untouched.

- **News headlines are clipped to 400 characters when read**, links to 1,000 and
  feed names to 80. See below for why; no real headline comes close.

### Fixed

- **A one-column terminal could bring the dashboard down.** The prompt dialog
  computed its cursor position as `popup.x + popup.width - 2`, which underflowed
  once the popup was clamped to a screen narrower than itself. Reachable by
  resizing your terminal while a prompt is open.

- **A news feed could decide how much work mirador does.** Nothing bounded the
  text read out of a feed, and everything downstream of a story runs on every
  frame — the headline is wrapped, the feed name uppercased. A 2 MB headline
  wrapped to 40,000 lines and cost **72ms a frame**, and the HTTP body limit
  allows five times that. A feed is the one input to this program that somebody
  else writes.

- **Place names and paths in non-Latin scripts drew over their own edges.** The
  text field measured its scroll window in *characters* rather than display
  cells, so a six-column field holding `北京市中心` decided five characters fit
  and drew ten cells, with the caret landing in the middle of a glyph.

- **A long line in a note body took the cursor off the screen.** The editor
  deliberately does not wrap — a soft wrap that moves as you type makes the
  cursor impossible to follow — and that had quietly come to mean you could keep
  typing past the right-hand edge and see none of it. It scrolls sideways now.

- **Arrange mode could rescale rows you had not touched.** Pushing a panel off
  the edge of a thin row grew the total of the layout weights, so a dashboard
  written as two equal rows became three unequal ones. Layouts written without
  explicit heights were the ones affected.

- **A list that got shorter left its cursor stranded.** Pressing Up walked the
  selection down from wherever it had been, one row at a time, with nothing
  highlighted the whole way. Down had always pulled it back into range; both do
  now.

- **`Alt` and a letter typed the letter into a note body**, where the task title
  field had always ignored it.

- **Moving the text cursor in the timezone picker threw away your place in the
  list.** Left and Right were handled as though you had typed.

- **A panel graph handed an area larger than the screen panicked** rather than
  drawing what fits, and a sample buffer given a capacity of zero spun for ever.
  Neither was reachable from any current caller; both are closed.

- **Rearranging a layout written with `[[layout.rows]]` sections** said only
  "no `[layout]` rows found", about a file that visibly has rows. It now names
  the form it can rewrite. That form is still the only one mirador edits.

## [0.15.0] - 2026-07-27

### Added

- **Six more themes, and a `t` key to try them on.** Nord, Gruvbox, Dracula,
  Catppuccin Mocha, Tokyo Night and Solarized Dark now ship inside the binary
  alongside mirador's own four, so the dashboard can match whatever your editor
  and terminal are already wearing. The values come from each palette's own
  specification and the theme file cites its source: these are ports, not
  interpretations.

  Press `t` to browse them. **The list previews as you move through it**, on
  your real dashboard rather than on a swatch, because a theme you cannot see is
  a theme you cannot choose. `Enter` keeps what is on screen; `Esc` puts back
  what you had. Themes of your own are listed alongside the shipped ones and
  marked as yours.

  Your choice is remembered the same way your weather units and sort order are.
  If you set `theme` in your config and it seems to be ignored, you picked
  something else with `t` at some point — pick it again, or delete the state
  file.

  All six keep `text = "reset"`, so body text still follows the foreground you
  have already tuned your terminal to.

### Fixed

- **A theme name is a name, not a path.** `theme = "../../elsewhere"` resolved,
  reading and parsing a file outside your themes directory. Nothing escalated —
  the config and anything it could reach are yours — but a name whose meaning
  depends on where your config sits is not a name.

- **Two mirador windows no longer take each other's saves away.** Every writer
  of a file used the same `.tmp` name, so whoever renamed second found it gone.
  Measured with eight concurrent writers: **2,100 of 2,400 saves failed** — and
  a failed save is reported, so the second window filled with "could not be
  saved" for no reason. Temporary names are now unique per write.

- **A file you restricted stays restricted.** A save replaces the file, and the
  replacement was created per the umask — so `chmod 600` on your tasks was
  silently widened to world-readable the next time you added one.

- **An untouched dashboard no longer writes to its state file.** `[agenda].file`
  ships commented out, so the baseline was empty while the panel reported a
  resolved path; one keystroke anywhere pinned that path into `state.toml`,
  after which setting `[agenda].file` in the config did nothing, because the
  state file outranks it. That is the failure invariant 17 exists to prevent,
  reached by a new route.

- **The agenda cloned its whole event list three more times per frame.** Once in
  `render`, once in `counter` — which the frame renderer calls on *every* frame
  with nothing guarding it — and twice more in key handlers that cloned the list
  only to read its length. Measured against a calendar of three hundred daily
  meetings: **210,000 event clones in thirty idle seconds**, now zero. A
  recurring rule expands, so the list is far longer than the file looks.

- **A `.ics` larger than 10MB is refused rather than read.** The network side
  was bounded and the local side was not, and reading a calendar costs more than
  its size — unfolding makes a `Vec<String>` of it and recurrence expands it
  again.

- **Editing a config on Windows no longer rewrites every line in it.** Both
  places that rewrite a file you wrote by hand — the layout editor and the
  config migrator — reassembled it from `str::lines()`, which strips the `\r`
  of a CRLF ending. Joining with `\n` then converted the whole file to LF: you
  moved one panel and git reported every line as changed. The ending the file
  already uses is preserved now, in both directions.

## [0.14.1] - 2026-07-27

### Fixed

- **A headline containing a wide character could freeze the dashboard.** Text
  wrapping split an over-long word by taking as many characters as fit — and for
  a CJK glyph or an emoji in a column one cell wide, none fit, so nothing was
  consumed and the loop ran for ever. Any news headline with such a character in
  a narrow panel would hang mirador until it was killed. **Present in 0.13.0 and
  0.14.0.**

- **The agenda cloned its whole event list on every frame.** The on-fire signal
  asks each panel whether anything is urgent, and the agenda answered by copying
  every event first. Measured: 456 event clones in thirty idle seconds against a
  twelve-event calendar, scaling with the size of the calendar. Now zero.

- **The news panel cloned every story on every frame**, for stories that change
  once an hour. Copied when the fetch lands instead.

- **The README said mirador has two network panels.** It has three.

## [0.14.0] - 2026-07-27

### Added

- **The status bar says when something needs you.** One line, naming the single
  most pressing thing — an event about to start, a save that is failing, a
  layout change that did not persist — and nothing at all when nothing is
  pressing.

  It names one thing and never a count, and it clears itself when the cause
  does. **There is no all-clear**: an indicator saying everything is fine is a
  light you have to read to learn nothing.

  Deliberately strict about what qualifies: *will this get worse if nobody acts
  in the next few minutes*. An overdue task does not — it is notable, already
  red in its own panel, and a signal lit for it would be lit permanently, which
  is how a warning becomes furniture.

### Fixed

- **A layout that could not be saved now says so.** It was reported inside the
  `w` picker and nowhere else, so a rearrangement that failed to persist was
  silent once the picker closed — and gone at the next launch. It was the one
  genuine silent failure left in the program.

## [0.13.0] - 2026-07-27

### Added

- **A news panel.** Headlines from RSS feeds you choose, refreshed hourly.

  **A window, not a feed:** however many stories fit and no more, with no
  scrolling, no count, no unread state and nothing to dismiss. News is the
  doomscroll surface this dashboard has been avoiding, and that commitment is
  what makes it something you glance at rather than something you work through.

  Stories are interleaved across feeds so the top of the panel holds the newest
  from *each* — date order alone hands the whole window to whichever outlet
  publishes most often.

  `o` shows a story's link so you can copy it; no browser is launched. The
  shipped feeds are science, space and technology only, because choosing
  outlets for general news is an editorial act this project should not make for
  you. Headlines only — feed summaries are article prose belonging to whoever
  wrote them.

- **The watch log records the day turning.** It is the one source that always
  fires, so the panel has something to say before you point it at a calendar,
  and it doubles as the divider marking where one day's entries end. Recorded by
  the shell rather than a panel: the todo panel notices a rollover too, but a
  day-divider that vanishes when you switch off the task list would be odd.

- **An empty watch log says what it is watching.** It had no refresh key —
  nothing there is polled, the panels report to it — and "Nothing has happened"
  on its own gave a reader no way to tell working from dead. It now names the
  two things it watches, and says plainly when no calendar is configured that
  only one of them can happen.

- **The default layout is four rows**, with news and the watch log sharing a
  reading row. Both want width for prose rather than columns of numbers, and
  squeezing them in beside the lists left neither readable.

## [0.12.0] - 2026-07-27

### Added

- **The version is shown in the `?` overlay**, right-aligned in its border the
  way every panel shows a counter. `?` is where you go to find out what the
  thing does, so it is where you look for what version it is.

- The command-line `--help` now leads with the version, and lists `w`, `m` and
  `Ctrl+arrows`, which it had never been told about.

### Changed

- **Adding a clock offers a list of cities instead of asking for an identifier.**
  Type to narrow it, `↑↓` to choose. Matching runs over the city *and* the
  identifier, anywhere in either, so `seattle` finds `America/Los_Angeles` —
  which is the whole point: the identifier names *a* city in the zone and it is
  very often not the one you have in mind. Bengaluru is `Asia/Kolkata`, Boston
  is `America/New_York`.

  The city you picked becomes the clock's label, and the identifier is shown
  beside it so what lands in `zones.toml` is never a surprise. A zone the list
  does not carry is still taken as typed.

### Fixed

- **A panel's prompt is no longer drawn inside that panel.** The agenda's file
  prompt was as narrow as the agenda, which for a long path meant reading a
  scrolled fragment through about forty columns. Prompts are drawn by the shell
  over the whole terminal now, after every panel — which is also what stopped
  the new city list coming out interleaved with the task list.

- **The city list scrolls.** It draws ten rows, and moving the selection past
  the bottom kept moving a cursor nobody could see — the highlight vanished and
  the row `Enter` would take was anybody's guess. The window follows the
  selection now, `PageUp`/`PageDown`/`Home`/`End` work, and the help line says
  how much of the list is on screen so ten of a hundred and forty-three does not
  read as all of it.

## [0.11.0] - 2026-07-27

### Added

- **The watch log** — a placeable panel recording what happened while you were
  not looking. It fills the third instrument the design thesis named
  (*"chronometer, weather glass, watch log"*) and that nothing had ever
  occupied.

  Almost nothing qualifies for it, which is the point. The clock, the readings,
  the prices and the graphs change continuously and none of it is news; your
  notes and tasks change because you changed them. An entry is something that
  happened *to* you rather than because of you, and that you would want to know
  even if you never looked at the panel it came from. Out of the box that is two
  things: an event appearing in your `.ics` that you did not add, and a task
  crossing into overdue because the day turned.

  **No counter, no unread state, no effect on any other panel.** Unread message
  counts were considered for this dashboard and rejected as a doomscroll hook;
  read closely, that objection is about the *badge* — a number that accumulates
  and demands you zero it. This is a record you consult, not an inbox that
  consults you, and nothing in it can be dismissed, because an entry you can
  dismiss is an entry you are expected to dismiss.

  A rule line marks where you were last seen. mirador now asks your terminal to
  report window focus, which is the only honest signal for "the reader is here";
  it falls back to your last keypress, and draws no line at all when neither has
  fired, because a line in the wrong place makes a claim that a missing one does
  not.

  The log lives in memory and says when it started watching. One written to disk
  would return after a restart with a gap it could not mark.

## [0.10.0] - 2026-07-27

### Added

- **Named themes.** `theme = "high-contrast"` in place of the `[theme]` table.
  Four ship inside the binary — `default`, `default-light`, `high-contrast` and
  `ansi` — and anything in `themes/<name>.toml` beside your config is found
  first, so a bundled theme can be replaced without renaming it.

  A theme file may `inherits` another, so a variant is the lines that differ
  rather than a copy of all eighteen keys, and may define a `[palette]` of named
  colours. Redefining a palette entry in a child recolours the keys its *parent*
  set with it.

  The same key does both jobs and TOML decides which: a quoted string is a name,
  a table is colours written out. Your existing `[theme]` table keeps working
  and keeps its error messages — the misspelled-key report and the
  `--migrate-config` hint for the pre-0.1.0 `rx`/`tx` keys both survive, which
  an untagged enum would have flattened into "data did not match any variant".

  Not built, deliberately: dotted-scope fallback (sized for Helix's hundreds of
  syntax scopes, where mirador has thirteen flat semantic keys) and per-key
  style objects with fg/bg/modifiers (every theme read takes a flat colour, and
  emphasis belongs to the widget that knows what it is emphasising).

- **A theme file with its colour keys after `[palette]` is refused by name.**
  TOML puts every key following a table header *inside* that table, so those
  keys set nothing — without failing, without a typo, and with the theme quietly
  coming out as the defaults. mirador's own `default.toml` was written that way
  during development and the test comparing it against the built-in default
  passed, because a file that sets nothing resolves to exactly the default.

## [0.9.1] - 2026-07-27

### Changed

- **Arrange mode says that rows can be opened.** The legend read
  `↑↓ move rows`, which a reader reasonably took to mean "move between the rows
  you have" — and then asked how to make a new one, which the mode had been
  able to do all along. It now reads `↑↓ at the edge opens a new row`, paid for
  by collapsing the two movement hints into one: which arrow goes which way is
  obvious the moment you press it, because the panels move.

  The legend also drops hints whole rather than clipping them when the terminal
  is narrow, keeping `Enter keep` and `Esc cancel` longest. Half a hint reads as
  a rendering fault; a missing one reads as a narrow terminal, which is what it
  is.

- The README now answers "how do I manage rows" directly instead of leaving it
  inside a sentence about moving panels.

## [0.9.0] - 2026-07-27

### Added

- **Arrange mode.** Press `m`, then move the focused panel with the arrows.
  `←`/`→` swap it with its neighbour; `↑`/`↓` move it between rows, landing it
  under wherever it already was rather than at the same index. Push it past the
  top or bottom edge and it takes a row of its own — which is how you get a
  fourth row without opening an editor — and the last panel out of a row closes
  that row. The real panels move as you press, `Enter` keeps it and `Esc` puts
  everything back. A panel keeps the width you gave it, and the row weights
  always add up to what they added up to before.

- **`Ctrl+arrow` resize is discoverable.** It has worked since long before this
  and only ever appeared in the `?` overlay. It is in the status bar now, and
  arrange mode's legend names it.

- **Panels can ask for a setting.** The agenda takes an `.ics` path with `f`,
  the weather takes a location with `L`, and the clock adds a zone with `a` and
  removes one with `d`. `Tab` completes — filesystem paths for the agenda,
  timezone names for the clock — as far as every candidate agrees. Each panel
  checks what it can before accepting: the agenda stats the file, the clock
  resolves the zone, and a refusal keeps what you typed so one wrong character
  does not cost you the whole path.

  Weather location was the oldest item on the open-work list.

- **World clocks are a data file.** `zones.toml` beside your tasks, the same
  arrangement as the watchlist and for the same reason: mirador never rewrites
  your config, so a list kept there could only be changed in an editor.
  `[clocks].zones` seeds your first run and is not read again.

### Fixed

- **A narrow panel no longer eats the bracket that closes its title.** At around
  100 columns the frame drew `╭┤9 CPU┤18 cores├╮` — the title and the counter
  are separate border segments and the title was being clipped, taking its `├`
  with it, which reads as a broken frame rather than a narrow one. The title is
  shortened instead, and below about 14 columns it is dropped rather than drawn
  as an empty `┤├`.

- **Reordering panels in the config actually works.** `layout_edit` matched
  panels by name and had no concept of order, so moving a panel along its row
  produced no edit at all and the safety check then refused a change that had
  visibly happened on screen. It could not open or close rows either. It can do
  all three now, and a moved panel takes the comments written above it along
  rather than leaving them to caption whatever slid into its place.

## [0.8.0] - 2026-07-27

### Added

- **An optional update check.** `[general].check_for_updates = true` asks
  crates.io, at most once a day, whether a newer mirador exists, and says so
  once in the status bar if it does.

  **Off by default and staying that way.** Everything else in mirador reaches
  the network only because a panel you placed needs data; this would reach it on
  its own behalf, telling a third party your IP address and that you run this
  program on a schedule you did not pick. Small, and still the thing "nothing
  phones home" was promising — so it is yours to turn on.

  When on: no identifier is sent, the answer is cached in `update-check.toml`
  beside your tasks so a day of restarts costs one request, a failure is silent,
  and the notice retires on your first keypress. `NO_UPDATE_CHECK=1` or
  `DO_NOT_TRACK=1` in the environment overrides the config.

- **The README now says how mirador is built.** It is written with heavy AI
  assistance, and anyone weighing up the code deserves to know that without
  having to infer it from the commit history.

## [0.7.1] - 2026-07-27

### Fixed

- The two links to dist's documentation pointed at `opensource.axo.dev`, which
  no longer resolves. They point at `axodotdev.github.io` instead.

## [0.7.0] - 2026-07-27

### Added

- **An agenda panel**, reading a local `.ics` file. The gap it fills was named
  early and left open for a long time: tasks are self-paced and a meeting is
  not, so a dashboard that could answer four questions and none of them was
  "you are in a call in ten minutes" was missing the one with a deadline.

  It is deliberately offline. mirador does not sign in to a calendar server —
  that means an account, a token to refresh, and a background process holding
  your credentials. Point `[agenda].file` at a calendar you already have and
  keep it current however you like; it is re-read on a timer and on `r`.

  Recurring events are expanded for the rules people actually have — daily,
  weekly with `BYDAY`, monthly, yearly, with `INTERVAL`, `COUNT`, `UNTIL` and
  `EXDATE`. Anything more elaborate shows only its first occurrence rather than
  guessing: a calendar that invents a meeting costs a wasted trip, where one
  that misses a repeat costs a glance at the real thing.

  Parsed in-tree with `jiff`, which mirador already has. `icalendar` and `rrule`
  would have added 30 crates and **2.3 MB** to a 3.4 MB binary — measured — most
  of it `chrono-tz` carrying a second timezone database.

  An unconfigured panel says "No agenda file" and how to set one, in ordinary
  colours — that is a panel nobody has set up, not a fault. A file that exists
  and cannot be read is the fault, and says so.

  **The default layout now places ten panels**, so the last of them has no
  number to jump to; `1`–`9` covers the rest and `Tab` reaches everything. The
  bottom row is reordered so the pomodoro keeps the width its numerals need.

## [0.6.0] - 2026-07-27

### Changed

- **`[cpu].sample_secs` and `[network].sample_secs` now default to 2.** Since
  the redraw follows visible change, every sample is a new number and every new
  number costs a repaint, so these two panels set the floor on what an idle
  dashboard does. Measured at 400x100 on the default layout, redraws per idle
  minute:

  |                        | `sample_secs = 1` | `= 2` |
  | ---                    | ---               | ---   |
  | `show_seconds = true`  | 95                | 81    |
  | `show_seconds = false` | 66                | 36    |

  The second row is the change. The first is what the clock costs: with seconds
  on it asks for a repaint every second and swamps the rest, which is worth
  knowing before blaming the graphs.

  The charts now cover twice the wall-clock time for the same buffer, and the
  span beside the figure says so. The cost is a two-second average, so a brief
  spike reads slightly lower — set it back to 1 if you would rather watch
  closely than leave it open all day.

  **Only new installs are affected.** The config seeds on first run and mirador
  never rewrites it, so an existing `config.toml` keeps whatever it already
  says.

## [0.5.2] - 2026-07-27

Documentation only; no code changed since 0.5.1.

### Changed

- The README opens on the demo recording alone. It previously showed a static
  screenshot of the same dashboard immediately above it, which made the same
  first impression twice; the recording is that screen plus what happens when
  you press something.

## [0.5.1] - 2026-07-26

### Fixed

- **Switching a panel on or off no longer disturbs the others.** Toggling
  anything in the `w` picker used to rebuild every panel, which reset a running
  pomodoro to 25:00 and sent the weather and market panels back to "loading" for
  a fetch cycle. Panels that are still placed are now carried across untouched.
  Keyboard focus follows its panel to wherever the new layout puts it, instead
  of staying on an index that may now be a different panel.

### Added

- A [demo recording](https://github.com/jchultarsky/mirador#readme) in the
  README, and `docs/record-demo.sh` that regenerates it from a real build.

## [0.5.0] - 2026-07-26

Fifteen items from an adversarial review of the whole project, worked in order
of how much each could cost someone.

**Upgrading:** two changes can stop a config that used to start. A misspelled
key under `[theme]` is now an error rather than being silently ignored, and an
absurd `[weather].refresh_minutes` or `[stocks].refresh_secs` is rejected rather
than wrapping. Both report the key and how to fix it. If your config was written
before 0.1.0, `mirador --migrate-config` will update it.

### Fixed

- **`Esc` no longer quits the dashboard.** It was in the global quit arm, so the
  reflex that closes a dialog closed the program — and any unsaved note went
  with it.
- **A long note can be scrolled to its end.** The body scrolled against its
  unwrapped line count, so wrapping hid the tail.
- **`Ctrl+S` and `Enter` save from either field of a form**, not just the one
  the cursor happened to be in.
- **Removing a symbol from the watchlist reports a failed save** instead of
  dropping the error.
- **The weather and stocks pollers stop when their panel goes away.** Neither
  loop had an exit, and the panel picker rebuilds every panel on every toggle,
  so a few passes over it left several threads polling the same endpoints —
  multiplying a request rate that `CLAUDE.md` claimed was enforced in code.
  Measured: 3 threads at rest, 3 after ten toggles.
- **mirador no longer panics at startup on Windows** on a machine up for less
  than a day. `Instant` there counts from boot, so back-dating one by 24 hours
  returned `None` and the `unwrap` fired before the terminal existed.
- **A preference can be un-set again.** Switching temperature units to metric
  and back left the state file insisting on metric, permanently. The comparison
  that decides what to record now happens once, against the config as it was
  read, rather than in each panel against the value it was built with.
- **`?` scrolls.** With the tasks panel focused on an 80x24 terminal the overlay
  had 29 rows of content and 22 rows to draw it in, so nine of that panel's
  seventeen key bindings — including `/`, `e`, `p`, `s` and `c` — were invisible,
  with nothing on screen to say so. Arrow keys, `PgUp`/`PgDn`, `Home` and `End`
  scroll it, and the footer shows where you are. On a terminal with room to spare
  nothing changes: any key still closes it.
- **A long task title no longer draws over the panel border.** Titles were
  truncated by counting characters against a budget measured in terminal cells,
  so every CJK character or emoji overflowed by one cell.
- **A misspelled key under `[theme]` is now reported** rather than accepted and
  ignored. This also un-blocked the `--migrate-config` hint for the pre-0.1.0
  `rx` and `tx` theme keys, which could never fire because those keys parsed
  cleanly.
- **The weather panel recovers from a failure to look up your location.** It used
  to end its fetch thread, then go on offering "r to retry" with nothing left to
  act on the key — most often after a laptop resumed before its Wi-Fi did.
- **A failed price fetch keeps the last price and labels it with its age.** One
  timed-out request used to blank the price, the change, the percentage and the
  sparkline together for a whole refresh interval; a fetch thread that stopped
  quietly showed confident numbers indefinitely. A retained price is now shown
  muted, with how old it is.
- Panel rectangles are indexed consistently, so a layout entry that builds no
  panel cannot shift every later panel onto its neighbour's rectangle — which is
  what mouse clicks are matched against.
- **A wide terminal no longer parks the whole row on one panel.** Past the point
  where a row's panels have all reached their useful maximum, the surplus was
  handed to whichever panel happened to be uncapped last. At 400 columns that
  gave the clock 302 of them — about 145 of which were blank, since its numerals
  stop growing at 158 — while the weather panel beside it sat at 51. The excess
  is now shared across the row.
- **The network panel's `SESSION` totals are the session.** They were the
  machine's since-boot counters, so on a laptop up three weeks the panel read
  128 GB within a minute of launch.
- **Holding `r` on the watchlist can no longer outrun the one-minute floor.**
  The limit was applied to the polling interval, and `r` bypassed the wait
  entirely — 8 requests in 2 seconds against a source that blocks by IP address.
- An absurd `refresh_minutes` or `refresh_secs` is rejected instead of wrapping
  into a tight loop against a free API.
- Panel width changes made with `Ctrl+arrow` are saved shortly after you stop,
  rather than only on a clean exit — closing the terminal window used to lose
  them.

### Changed

- **mirador redraws when something changes, rather than when a timer fires.**
  Measured on the default layout at 400x100: **243 redraws a minute before, 62
  after** — and before, the number was the same whether `show_seconds` was on or
  off, so turning seconds off changed what was on screen and not what the
  program ran. Battery life on a dashboard you leave open all day is the point.
- Every file mirador writes — including your config — now goes through one
  atomic write that flushes to the disk before replacing the original. The
  config was previously overwritten in place.
- The README leads with what mirador is for rather than with a feature
  comparison, and says plainly what using Yahoo's undocumented chart endpoint
  does and does not mean for you.

### Security

- Release artifacts carry GitHub build provenance attestations, and the shipped
  binaries embed their dependency list so `cargo audit bin` can check them.
- `main` requires all eight CI jobs, including Windows and a `cargo-deny` supply
  chain check. CI actions are pinned by commit rather than by tag.
- Documented honestly what the one-line installers verify — the shell one checks
  the archive's sha256, the PowerShell one checks nothing, and neither checks
  the updater. See [SECURITY.md](SECURITY.md#verifying-a-download).
- Private vulnerability reporting is enabled, which the security policy had been
  pointing people at while it was switched off.

## [0.4.0] - 2026-07-26

### Added

- **A panel picker.** `w` opens a dialog listing every widget with whether it is
  on; `space` toggles, and the panel appears or disappears immediately.
  Switching a widget on used to mean finding your config file and editing
  `[layout]` by hand — which is what the first person to meet the pomodoro
  panel actually had to do, after pressing `?` and not finding the answer there
  either. The status bar notice and the help overlay both name the key now.

  Panel sizes are remembered too: `Ctrl+arrow` no longer lasts only for the
  session.

### Changed

- **Layout changes are written back into your config**, reversing an earlier
  decision to keep them out of it. The rule was never really "do not write to
  the config" — it was "do not *reserialise* it", because a round trip through
  `toml` discards every comment in the file, including the ones mirador wrote to
  explain its own options. `--migrate-config` had already established the
  alternative: edit the lines that need editing and leave the rest alone.

  So `[layout]` is edited surgically. Adding a panel is a one-line diff; a width
  change rewrites one number and keeps its column alignment; comments inside the
  layout block survive.

  The safety property is a check rather than care: the edited text is parsed and
  compared against the layout that was asked for, and a mismatch throws the edit
  away. An unusually formatted config fails as "that did not stick", said in the
  picker, rather than as a broken file.

  Preferences that are not layout stay in the state file. The split is by what
  the setting *is*: `[layout]` is the part of the config people read and curate,
  so a change made in the UI has to show up there, while nobody keeps their
  preferred sort order under version control.
- Windows is described as working rather than as untried. The binary shipped in
  0.2.0 was built and packaged but had never been started on the platform, and
  the docs said so; it has now been run and works, installed with the PowerShell
  one-liner in the default Windows terminal. All three shipped targets have been
  started rather than merely compiled, and that is also the first real-world use
  of the PowerShell installer — the rest had only been checked from macOS. It is
  still the least-travelled of the three, which the README says instead of
  claiming a parity nobody has earned.

## [0.3.1] - 2026-07-25

### Fixed

- The README on crates.io no longer contradicts its own screenshot. The image
  is referenced by absolute URL so that it resolves on crates.io as well as
  GitHub — which means it tracks `main` and updated the moment a new capture
  landed, while the caption around it stayed frozen at whatever the last
  publish baked in. The published page ended up showing the pomodoro panel
  above a caption explaining that the shot predates it. The alt text was stale
  for the same reason, which matters more, since a screen reader has only that.

  Worth knowing for any future image: a floating URL and fixed prose drift
  apart by design, so a caption describing the picture has to ship in the same
  release as the picture.

## [0.3.0] - 2026-07-25

### Added

- `mirador-update`, installed beside the binary by the shell and PowerShell
  installers. It asks GitHub for the newest release and installs it if that is
  newer, so upgrading no longer means going back to the releases page. It runs
  only when invoked: mirador itself never checks for updates and does not know
  the program exists, which keeps the "no telemetry" line in the README exactly
  as true as it was. Anyone who installed with `cargo install` or from source
  upgrades the way they installed.
- **Settings changed from the keyboard are remembered across restarts.**
  Weather units, the task sort order, whether completed tasks show, seconds on
  the clock, and the pomodoro durations. The watchlist already did this for
  symbols; this is the same answer generalised.

  mirador still never rewrites your config — the property that makes it safe to
  hand-edit and keep in git. The config *seeds* a setting and a small state
  file beside your tasks records where you moved it since. Deleting that file
  puts everything back, which the file's own header says, because a preference
  you cannot remember setting needs an obvious way out.

  **Only what you actually changed is written.** Pressing `u` records the units
  and nothing else. The first version reported every panel's current value,
  which pins the config's own settings into the state file the moment any one
  preference moves — after which editing the config silently stops working.
  That passed its tests and was caught by running it.

  Panel sizes are deliberately excluded: `Ctrl+arrow` resizing is geometry
  rather than preference, and remembering it would mean a saved width quietly
  overruling a `[layout]` edited by hand.
- **Pomodoro panel.** A focus timer in the same block numerals the clock uses:
  `space` starts and pauses, `n` skips a phase, `r` restores the current one,
  and `+`/`-` change the length of the phase you are in. Focus is brass and
  breaks are moss, with the phase named above the numerals as well, because
  colour alone is a poor way to tell someone whether they are meant to be
  working. A paused timer greys out rather than blinking — this is a dashboard
  you leave open, and a flashing clock is the opposite of that.

  `+` and `-` move the phase length and the time left together, so adding a
  minute part-way through does not rewind you to the start. A phase that ends
  while you were away advances exactly one step against a fresh clock rather
  than chaining from the deadline it missed, so a laptop resumed after an hour
  does not race through six phases catching up.

  An optional chime, **off by default**, rings when a phase ends. With no
  command configured it is the terminal bell, which lets your terminal decide
  whether that means a sound, a flash, or nothing. mirador ships no audio
  library on purpose: playing a file means linking the platform audio stack,
  and on Linux that is a C library and its dev headers on every builder — a
  large amount of machinery for one notification. `chime_command` names a
  player instead, run directly with no shell in the way, and a player that
  cannot start is reported in the panel rather than silently doing nothing.

### Fixed

- The pomodoro panel stops claiming space it cannot use. It declared a maximum
  of 102 columns and 21 rows — the width of the numerals at the largest scale
  the clock allows — and on a wide terminal it took them, from the task list.
  The numerals are now capped at the scale where `MM:SS` is 38 columns by 5
  rows, which is already a chunky readout; scale 2 is 68 columns and scale 3 is
  98, and a timer occupying half a dashboard reads as an alarm rather than an
  instrument. The panel now declares 42 by 10 and hands everything past that to
  a neighbour, and the figures are pinned by a test so they cannot drift back.
- The README says what happens when you upgrade. A new widget does not appear
  in a config you already have — mirador never rewrites your config, which is
  what makes it safe to hand-edit, so a config written by an earlier version
  lays out the panels that existed when it was written and nothing since. The
  status bar has always named the widgets a layout does not place; the README
  now explains the notice, shows what it looks like, and gives the row to paste.
  The first person to add the pomodoro panel to an existing config went looking
  for a missing setting instead.
- The README's checksum command no longer prints a warning. `dist` writes its
  `.sha256` files with a trailing blank line, so `shasum -c` verified the
  archive and then added `WARNING: 1 line is improperly formatted` — true of
  the blank line, not of the hash, but a warning printed beside a checksum is
  the one place ambiguity is worst. Piping through `grep .` drops the blank
  line; both the macOS and Linux forms are tested to print `OK` and exit 0 on a
  good archive and `FAILED` and exit 1 on a tampered one. A Windows form is
  documented too, now that there is a Windows archive to verify.

## [0.2.0] - 2026-07-25

### Added

- Windows binaries. Releases now carry `x86_64-pc-windows-msvc` alongside the
  two macOS targets and Linux x86-64, plus shell and PowerShell installers and
  a source archive, all with checksums.

  `aarch64-pc-windows-msvc` is deliberately absent: `ring`, reached through
  `ureq`'s TLS, does not build for it. musl is absent for the same reason. The
  binary is built rather than exercised — nothing in mirador is Unix-specific
  and its dependencies are cross-platform, but that is a claim about compiling,
  not about behaving, and only the first has been checked.

### Changed

- `.github/workflows/release.yml` is generated by `cargo dist` and is no longer
  hand-edited; it is build output. The tag-versus-manifest check and the
  prerelease detection added a release ago are gone from it because `dist`
  provides both natively, and archive names lose their embedded version
  (`mirador-aarch64-apple-darwin.tar.gz`).

  `dist init` also wanted `[profile.dist] lto = "thin"`, which would have
  quietly undone the `lto = true` the release profile has always had. That
  override is removed, so the binaries do not get slower for changing how they
  are packaged.

## [0.1.0] - 2026-07-25

Initial release.

The Changed and Fixed sections record decisions reversed and defects found
*before* this first tag rather than after it. Nothing below was ever shipped
in an earlier version — they are kept because the reasoning is worth having.

### Added

- First run is no longer blank. The task list and notes seed a few examples
  when their file does not exist yet, and the examples are the documentation:
  they name the keys, carry a due date in each direction, a priority, a tag and
  a note, so the table has something to line up and an overdue row shows what
  overdue looks like. They are ordinary entries and `d` deletes them.

  Keyed on the file being *absent* rather than empty, so clearing them sticks —
  the opposite would make them impossible to get rid of. Their titles are held
  to what the task column can show at 120 columns, since an instruction
  truncated to `Press ? for every key, h…` is worse than no instruction.
- A startup hint naming widgets your layout does not place, on the right of the
  status bar, plus a section in the help overlay. A config written by an earlier
  version silently lacks every widget added since — an absent widget is a valid
  choice, so nothing errors and `--migrate-config` has nothing to fix, which
  left reading the release notes as the only way to discover a new panel. The
  status-bar notice is retired by the first keypress or click, because a
  dashboard you leave open all day must not nag; the help overlay keeps it, on
  the grounds that `?` is where you go when you wonder what else there is.
- Stock watchlist panel: last price, the day's change in currency and percent,
  and an intraday sparkline. `a` adds a symbol, `d` removes one, `r` refreshes.
  The watchlist is stored as a data file rather than in the config, which is
  what lets the panel edit it — mirador deliberately never rewrites the config,
  so a watchlist living there could only ever be changed in an editor. Config
  seeds the first run and nothing after it.

  The quote source is a trait from the first commit rather than a later
  refactor: the default gates on IP reputation instead of on a key and blocks
  datacenter and VPN ranges outright, so the same build works on a laptop and
  fails on a VPS. That can only be routed around by swapping the source. No API
  key is bundled, polling is clamped to once a minute, symbols are fetched one
  at a time rather than in a burst, and prices are never written to disk.
- Notes panel: free-form notes with a title, a body and the dates they were
  written and last edited. Master-detail, the shape a mail client uses — the
  list of titles and dates sits beside the selected note's body, because a
  note's whole value is the text inside it and making the reader press a key to
  see any of it turns "glance at the dashboard" into "operate the dashboard".
  The split follows the panel: side by side when there is width for both,
  stacked when there is not. `/` searches bodies as well as titles, since the
  title you wrote in a hurry is often not what you later search for. Notes are
  stored as hand-editable TOML and written atomically, like tasks.
- Calendar panel: month grids in the shape `cal` prints, showing the current
  month and the next, with today marked in reverse video. `n`/`p` step a month,
  the arrows step a month or a year, `t` returns to today, and the wheel steps
  a month. Months lay out side by side when the panel is wide and stack when it
  is tall; a month block is always six week-rows high, so scrolling does not
  make the panel jump.
- Mouse support, off the `[general].mouse` switch. Clicking a panel focuses it;
  scrolling the wheel moves the list or calendar under the pointer *without*
  taking focus, so the wheel cannot steal the keyboard from what you were
  doing. A panel in a text-entry state vetoes mouse actions exactly as it
  vetoes global keys, so a stray click cannot strand a half-typed task.
- Panels resize from the keyboard, the way tmux panes do: `Ctrl+←/→` trades
  width with the neighbouring panel in the row, `Ctrl+↑/↓` trades height with
  the neighbouring row. The total is held constant so widening one panel
  narrows exactly one other and nothing else on screen reflows, and no panel
  can be squeezed to nothing — a panel with no width could never be focused to
  get its space back. Resizes last for the session; the config is not rewritten.
- The dashboard now redraws only when something changed, rather than once per
  event loop pass. Mouse reporting made this necessary — terminals send an
  event for every cell the pointer crosses — but it also means an idle
  dashboard left open all day costs a redraw a second instead of four.

- Column grid with named headers, shared by every panel that lists things.
  Tabular data now reads as a table: fixed column positions, right-aligned
  numbers and dates, and headers in the letterspaced utility face. Optional
  columns drop whole rather than squeezing when a panel is narrow.
- Braille history graphs at 2 samples per cell and 4 levels per row, with the
  colour gradient running vertically by magnitude. The profile is static as
  data scrolls, so a graph left on screen all day does not shimmer.
- Three-stop colour gradients, baked to a lookup table at startup and shared
  between a panel's graph, meter and numeric readout.
- Key hints drawn into the focused panel's bottom border, the panel's jump key
  in its title, and a status counter in the top-right — all costing zero
  interior rows. Hints are shown only for the focused panel.
- `Binding` type: one declaration feeds the border hint, the status bar and the
  help overlay, so hints cannot drift from the keys they describe.
- Scalable block numerals for the clock, sized to the panel. Seconds ride small
  beside the large `HH:MM` when the full time will not fit.
- Weather art for ten sky conditions.

- Terminal dashboard with a configurable grid layout. Rows and panels use
  relative weights, so a layout keeps its proportions at any terminal size.
- `Panel` trait as the extension seam; each widget owns its own state and
  refresh cadence, and the application shell handles only layout, focus and
  event dispatch.
- **Clocks** widget: any number of world clocks by IANA timezone, with the
  literal `local` for the system zone. Configurable time and date formats, and
  optional UTC offsets. Unresolvable zones are reported inline rather than
  dropped.
- **Weather** widget: current conditions plus a 1–7 day forecast from
  Open-Meteo, which needs no API key or account. Location is geocoded from a
  place name, or given directly as latitude and longitude. All network I/O runs
  on a background thread, so a slow request never blocks rendering.
- **To-do** widget with full create, read, update and delete:
  - Tasks carry a title, notes, due date, priority, tags and completion state.
  - Add and edit through an in-panel form; delete asks for confirmation.
  - Due-date input accepts ISO dates, `today`/`tomorrow`, weekday names, and
    offsets such as `+3d` or `2w`. Unrecognised input is rejected with an
    explanation.
  - Five sort modes, including a "smart" default that surfaces overdue work
    ahead of high-priority work that is not yet due.
  - Live filtering across titles, tags and notes.
  - Storage is a plain, hand-editable TOML file. Writes are atomic, and save
    failures surface in the panel rather than being swallowed.
- **CPU** widget: average utilisation as a scrolling chart, with per-core
  meters and configurable warning and critical thresholds.
- **Network** widget: receive and transmit throughput as scrolling charts, with
  session totals. Rates are derived from real elapsed time between samples, so
  a delayed tick reports the correct rate rather than an inflated one.
- Configuration in a single TOML file, written with full comments on first run.
  Unknown widget names, malformed colours and invalid units are rejected at
  startup with actionable messages.
- Command-line flags: `--config`, `--config-path`, `--print-config`, `--help`
  and `--version`.
- Help overlay bound to `?`, listing global bindings and those of the focused
  panel.

### Changed

- The weather panel declares a maximum width and hands the surplus to the
  clock. Once every forecast column is showing, more width only inflates the
  flexible sky column and pushes the numbers away from the labels they belong
  to. On a 193-column terminal the clock goes from 57 columns to 80 — enough to
  render block numerals where it had been falling back to plain text, which is
  the one thing that panel exists not to do.
- The weather forecast fills the height it is given: `[weather].forecast_hours`
  is a floor the panel is sized for, not a ceiling on what it may show, and the
  fetch retrieves a full day so a taller panel never waits for a refetch.
- The calendar stacks further rows of months into spare height, up to a year on
  screen. `[calendar].months` is now how many sit *across* — and so how wide the
  panel ever gets — rather than how many exist; it is a floor on the number
  shown, not a ceiling. The width cap stays, so the panel still hands surplus
  columns to its neighbours instead of spreading out.
- The stock watchlist declares a maximum width. Every column but the sparkline
  is fixed and the sparkline is capped, so past that the table only drifts
  apart; the columns go to the CPU and network graphs, which have no such limit.
- Panels may declare the size past which more space does nothing for them, and
  the layout hands their surplus to a neighbour that can use it. A clock cannot
  use a hundred columns and a calendar cannot use more than its months need;
  proportional layout gave them the space anyway and they sat in it while the
  task list next door ran out. The calendar and clock are bounded on both axes,
  the weather panel on height. When every panel in a row is bounded the maxima
  are ignored rather than leaving a gap — panels draw their own frames, so
  unallocated cells would show as a hole.
- `[weather].units` switches at runtime with `u`. The conversion is applied at
  render rather than re-requested, so the change is instant instead of putting a
  network round trip behind a keypress, and the forecast table converts with the
  readout above it.
- The note body sits below the list rather than beside it. Side by side splits a
  finite width between a list that wants room for titles and a body that wants
  room for prose, so neither got enough; stacking gives both the full width and
  spends height, which is the cheaper axis for both. `[notes].preview` takes
  `below` or `beside`, replacing `side_by_side_min_width`.
- CPU and network history buffers grow to fill the panel. `history` is a floor
  now, not a ceiling: the graphs pack two samples per cell and fill from the
  right, so a buffer of N samples could only ever cover N/2 cells and anything
  wider showed dead space on the left. The span readout is computed from the
  live sample count, so it stays honest as the buffer grows.

- The crates.io name is reserved: `mirador` `0.0.0` is published. Reservation is
  first-come with no reclamation and the name is an ordinary English word, so
  this was the one outstanding item with a deadline attached rather than a
  preference. `0.0.0` says reservation rather than release, which leaves `0.1.0`
  free for the first real one. The README says so plainly instead of letting the
  version number imply that a placeholder is a shipped product, and MSRV and
  platform badges now sit alongside the crates.io one.
- Labels are bold uppercase rather than letterspaced: `NEXT HOURS`, not
  `N E X T  H O U R S`. Tracking was meant to read as an engraved instrument
  label and instead read as stretched text, and it more than doubled the width
  of every label — which is why the grid header carried logic to demote a whole
  row to plain caps when one column could not fit. That logic is gone with it.
- The weather forecast is hourly rather than daily, and every row is labelled
  with the hour it applies to.
- The clocks panel renders the first configured zone large, with the rest as a
  labelled table showing offsets *relative to that zone* rather than to UTC.
- Panels have one column of interior padding.
- Focus is signalled by dimming unfocused panels rather than brightening the
  focused one, so exactly one thing on screen is at full brightness.
- The default palette is brass, verdigris and slate rather than cyan. Body text
  is `reset`, inheriting the terminal's own foreground.
- Seconds are shown in the clock by default.

### Fixed

- A config with no `[layout]` section no longer silently loses three panels.
  The Rust default described a five-widget dashboard while the shipped file
  described eight, so deleting a section you thought was redundant quietly took
  the calendar, notes and watchlist with it. They now describe the same
  dashboard, and a test fails if they drift apart again.
- The minimum supported Rust version is 1.95, not 1.85. The declared floor had
  stopped being true: `sysinfo` requires 1.95 and ratatui's tree requires 1.88.
  `Cargo.toml`, the CI toolchain, the README and CONTRIBUTING now agree.
- A rustdoc intra-doc link pointed at a `cfg(test)` item, which rustdoc cannot
  resolve; with `-D warnings` that failed the documentation build.
- Dependabot no longer proposes a Rust version that does not exist.
  `dtolnay/rust-toolchain` publishes a ref per Rust release rather than semver
  tags, and dependabot ordered them numerically: it read the MSRV job's
  `@1.95.0` pin and offered `@1.100.0`, which fails to install with a 404 from
  `static.rust-lang.org`. That pin tracks `rust-version` by hand and the other
  five uses are `@stable`, so version updates to that action are now ignored
  outright rather than closed by hand every month.
- Deleting a task no longer hands its id to the next one added. Ids came from
  `max(id) + 1`, so removing the highest-numbered task gave that number
  straight back, and anything still holding it — the selection, an open edit
  form, a pending delete confirmation — would silently act on a different task.
  The store now keeps a high-water mark that only climbs, rebuilt from the file
  on load, which is safe because nothing holds an id across a restart.
- A dropped column no longer slides every value after it under the wrong
  header. Row cells were indexed by *surviving* column position while callers
  pass them in declared order, so a narrow forecast showed the feels-like
  temperature under RAIN, and a narrow task list would have shown tags under
  DUE. Silent, and exactly the failure this module exists to prevent.
- Forecast columns appear as soon as they fit. Their thresholds were hand-tuned
  for a layout that no longer exists and were about fifteen columns too
  conservative — the table only completed at 62 when it fits at 47. They are
  now derived from one rule: a column appears once the grid can seat it and
  still leave the sky column its longest label. WIND now arrives at 47 rather
  than 62, RAIN at 33 rather than 38, FEELS at 39 rather than 52, and the
  weather panel's maximum width drops from 66 to 51, handing the difference to
  the clock.
- A failed weather refresh no longer blanks the panel. It used to replace the
  last good reading with an error, so on a dashboard left running for days —
  where a transient network blip is close to certain — one timed-out request
  cost the whole panel for a full refresh interval. The reading is kept and its
  age is shown instead: the border counter switches from the observation time
  to `2h old`, and the panel says so in its own body in amber. A reading is also
  called stale after twice the refresh interval even when nothing has failed,
  which catches a fetch thread that quietly stopped or a laptop resumed from
  sleep. Only a panel that has never loaded anything shows an error.
- The watchlist sparkline came back. Bounding the panel's width left the grid
  one column short of the threshold at which the sparkline column earns its
  place, so the column was dropped and the space sat empty. The threshold, the
  panel's maximum width and the width the sparkline is drawn at are now derived
  from one constant, and the panel asks the grid for the resolved column width
  instead of recomputing it — three copies of that arithmetic is what let them
  drift apart in the first place.
- Month names are centred over their own month. `centred` padded on the left
  but not the right, so a title line was short of the full block width and
  every month after the first slid left by the shortfall.
- The notes list and the reading pane have a rule between them. Without one the
  panel read as a single list whose last rows had gone strange: the two halves
  are the same kind of text in the same colours, so nothing else separated them.
- `Enter` on a task opens it for editing rather than marking it done. Enter
  means "go inside" everywhere else, so binding the most reflexive key in the
  list to a state change on the highlighted row was a trap. Completing a task
  is `space`, which reads as a checkbox.
- The task panel's key list now matches the keys it actually handles. `Enter`,
  `n`, the arrow keys, `Home`/`End`, `PgUp`/`PgDn` and `Esc` all worked but
  appeared nowhere in the help, so the only way to find them was to read the
  source. A test now pairs every handled key with the binding that documents
  it, and fails if either side gains an entry the other lacks.
- Unknown keys in the config file are now rejected at startup instead of being
  silently ignored. A config written by an older version used to load with its
  stale keys quietly dropped, which made current code look like a stale build.
  Keys renamed since 0.1.0 name their replacement in the error.
- `--migrate-config` updates a config written by an older version in place,
  keeping a `.bak` of the original. The migration is textual, so comments and
  formatting survive; it refuses to write anything that would not load.
- Text in tables is measured in terminal cells rather than characters. Glyphs
  like `☀` occupy two cells, so counting characters shifted every column after
  them and put values under the wrong headers.
- Weather marks use only emoji whose measured width matches what terminals
  draw; several obvious ones (rain cloud, snow cloud, cloud, fog) report one
  cell and render two. A test asserts this.
- Forecast cells always show a value: `0%` rather than a blank, which reads as
  a broken column.
- Task rows no longer shift horizontally when a task has no due date.
- Key hints are no longer duplicated between the panel body and its frame.

[Unreleased]: https://github.com/jchultarsky/mirador/compare/v1.20.0...HEAD
[1.20.0]: https://github.com/jchultarsky/mirador/compare/v1.19.2...v1.20.0
[1.19.2]: https://github.com/jchultarsky/mirador/compare/v1.19.1...v1.19.2
[1.19.1]: https://github.com/jchultarsky/mirador/compare/v1.19.0...v1.19.1
[1.19.0]: https://github.com/jchultarsky/mirador/compare/v1.18.0...v1.19.0
[1.18.0]: https://github.com/jchultarsky/mirador/compare/v1.17.0...v1.18.0
[1.17.0]: https://github.com/jchultarsky/mirador/compare/v1.16.0...v1.17.0
[1.16.0]: https://github.com/jchultarsky/mirador/compare/v1.15.0...v1.16.0
[1.15.0]: https://github.com/jchultarsky/mirador/compare/v1.14.0...v1.15.0
[1.14.0]: https://github.com/jchultarsky/mirador/compare/v1.13.1...v1.14.0
[1.13.1]: https://github.com/jchultarsky/mirador/compare/v1.13.0...v1.13.1
[1.13.0]: https://github.com/jchultarsky/mirador/compare/v1.12.1...v1.13.0
[1.12.1]: https://github.com/jchultarsky/mirador/compare/v1.12.0...v1.12.1
[1.12.0]: https://github.com/jchultarsky/mirador/compare/v1.11.0...v1.12.0
[1.11.0]: https://github.com/jchultarsky/mirador/compare/v1.10.2...v1.11.0
[1.10.2]: https://github.com/jchultarsky/mirador/compare/v1.10.1...v1.10.2
[1.10.1]: https://github.com/jchultarsky/mirador/compare/v1.10.0...v1.10.1
[1.10.0]: https://github.com/jchultarsky/mirador/compare/v1.9.0...v1.10.0
[1.9.0]: https://github.com/jchultarsky/mirador/compare/v1.8.0...v1.9.0
[1.8.0]: https://github.com/jchultarsky/mirador/compare/v1.7.0...v1.8.0
[1.7.0]: https://github.com/jchultarsky/mirador/compare/v1.6.1...v1.7.0
[1.6.1]: https://github.com/jchultarsky/mirador/compare/v1.6.0...v1.6.1
[1.6.0]: https://github.com/jchultarsky/mirador/compare/v1.5.1...v1.6.0
[1.5.1]: https://github.com/jchultarsky/mirador/compare/v1.5.0...v1.5.1
[1.5.0]: https://github.com/jchultarsky/mirador/compare/v1.4.1...v1.5.0
[1.4.1]: https://github.com/jchultarsky/mirador/compare/v1.4.0...v1.4.1
[1.4.0]: https://github.com/jchultarsky/mirador/compare/v1.3.2...v1.4.0
[1.3.2]: https://github.com/jchultarsky/mirador/compare/v1.3.1...v1.3.2
[1.3.1]: https://github.com/jchultarsky/mirador/compare/v1.3.0...v1.3.1
[1.3.0]: https://github.com/jchultarsky/mirador/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/jchultarsky/mirador/compare/v1.1.3...v1.2.0
[1.1.3]: https://github.com/jchultarsky/mirador/compare/v1.1.2...v1.1.3
[1.1.2]: https://github.com/jchultarsky/mirador/compare/v1.1.1...v1.1.2
[1.1.1]: https://github.com/jchultarsky/mirador/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/jchultarsky/mirador/compare/v1.0.4...v1.1.0
[1.0.4]: https://github.com/jchultarsky/mirador/compare/v1.0.3...v1.0.4
[1.0.3]: https://github.com/jchultarsky/mirador/compare/v1.0.2...v1.0.3
[1.0.2]: https://github.com/jchultarsky/mirador/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/jchultarsky/mirador/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/jchultarsky/mirador/compare/v0.19.0...v1.0.0
[0.19.0]: https://github.com/jchultarsky/mirador/compare/v0.18.2...v0.19.0
[0.18.2]: https://github.com/jchultarsky/mirador/compare/v0.18.1...v0.18.2
[0.18.1]: https://github.com/jchultarsky/mirador/compare/v0.18.0...v0.18.1
[0.18.0]: https://github.com/jchultarsky/mirador/compare/v0.17.1...v0.18.0
[0.17.1]: https://github.com/jchultarsky/mirador/compare/v0.17.0...v0.17.1
[0.17.0]: https://github.com/jchultarsky/mirador/compare/v0.16.3...v0.17.0
[0.16.3]: https://github.com/jchultarsky/mirador/compare/v0.16.2...v0.16.3
[0.16.2]: https://github.com/jchultarsky/mirador/compare/v0.16.1...v0.16.2
[0.16.1]: https://github.com/jchultarsky/mirador/compare/v0.16.0...v0.16.1
[0.16.0]: https://github.com/jchultarsky/mirador/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/jchultarsky/mirador/compare/v0.14.1...v0.15.0
[0.14.1]: https://github.com/jchultarsky/mirador/compare/v0.14.0...v0.14.1
[0.14.0]: https://github.com/jchultarsky/mirador/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/jchultarsky/mirador/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/jchultarsky/mirador/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/jchultarsky/mirador/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/jchultarsky/mirador/compare/v0.9.1...v0.10.0
[0.9.1]: https://github.com/jchultarsky/mirador/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/jchultarsky/mirador/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/jchultarsky/mirador/compare/v0.7.1...v0.8.0
[0.7.1]: https://github.com/jchultarsky/mirador/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/jchultarsky/mirador/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/jchultarsky/mirador/compare/v0.5.2...v0.6.0
[0.5.2]: https://github.com/jchultarsky/mirador/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/jchultarsky/mirador/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/jchultarsky/mirador/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/jchultarsky/mirador/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/jchultarsky/mirador/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/jchultarsky/mirador/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jchultarsky/mirador/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jchultarsky/mirador/releases/tag/v0.1.0

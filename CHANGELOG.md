# Changelog

What changed in each release, in plain English. Newest release first. Every
version tag needs an entry here before it can be released.

## v0.1.5 - 2026-10-12

- Added support for Gauntlet: Glitched. The header now shows the mode name, and all
  16 players appear in one list, with duo partners next to each other and your own
  duo first.
- Improved modes with more than two teams in general (this should also cover
  Deathmatch): players are no longer split into a wrong "Your team" / "Enemy team".
- Added the game server to the header during a match, using the same names as the
  in-game server picker, for example "US West (N. California)".
- Fixed Gauntlet: Glitched portraits showing "?". They now show the team-coloured
  KAY/O, like the in-game scoreboard.

## v0.1.4 - 2026-09-23

- Fixed headshot % and KD sometimes showing N/A for you (or others) when queuing
  right after a match. They now use the matches that could be loaded instead of
  giving up.
- Improved the last 5 results: wins, losses and draws now come from the actual
  match result instead of being guessed from RR changes, and draws have their
  own color.
- Added a hint when headshot % or KD is based on fewer than 5 matches: the number
  is dimmed and hovering it tells you how many matches it covers.
- Improved tooltips: hover text now appears in a cleaner in-app style instead of
  the plain system tooltip.

## v0.1.3 - 2026-09-04

- Added a "Copy diagnostics" link on the waiting screen and in the header during a match.
  If the tracker does not pick up your match, click it and paste the short report into a
  GitHub issue; it contains no passwords, tokens or full player ids.
- Added a "Report a bug" link next to it on the waiting screen. It opens the GitHub bug form
  with your app version already filled in; paste the diagnostics report into the form.

## v0.1.2 - 2026-08-26

- Added a prominent update button on the waiting screen so you can't miss when
  a new version is available. The small header chip still appears during matches.
- Added the PolyForm Noncommercial license.
- Improved the release pipeline by upgrading to current GitHub Actions versions.

## v0.1.1 - 2026-08-26

- Improved the release tooling behind the scenes. The app itself works the same
  as v0.1.0.
- First release delivered through the built-in updater: if you are on v0.1.0,
  the app should offer this update on its own.

## v0.1.0 - 2026-08-26

- Added the in-match player table: everyone in your game with their current rank,
  RR, peak rank, account level and agent.
- Added per-player stats alongside the ranks: win rate, headshot percentage, KD
  and the results of their recent games.
- Added weapon skin info, so you can see what each player is carrying before the
  round starts.
- Added live updates during agent select, so picks and locks appear as they
  happen instead of only once the match starts.
- Added a last-match view you can open from the menus, for a look back at the
  game you just finished.
- Added a built-in updater: the app checks GitHub for a newer version and can
  download and install it for you, no manual reinstall needed.

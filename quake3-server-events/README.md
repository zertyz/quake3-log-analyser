This crate represents an external library, agnostic of our requirements.

Its solo purpose is to "parse quake3 server log files".

For determining its own requirements and assist in the development/testing directions, the following analysis was conducted on the provided sample log file:

1) Each log line follows a well defined structure: TIME <space> EVENT_NAME: DATA
2) Comment lines are in a special form: TIME <space> <several dashes>
3) TIME is in the form HH:MM, where HH is in the 24 hour format
4) DATA has 3 forms: a map: 'name1\val1\name2\val2\...', a scalar value or custom contents
5) Each log file may contain several game matches in it, enclosed in the `InitGame` and `ShutdownGame` events
6) Each match may be either in "Capture the flag" or "Deathmatch" mode
7) A match may end "according to plan" or due to other reasons. In the former, the game server outputs log lines with the `score` of each player before the `ShutdownGame` event; in the latter, those `score` log lines are not produced (indicating the the match didn't end due to reaching any predefined limits, such as the "fraglimit" or "capturelimit" -- for the "Deathmatch" and "Capture the flag" modes, respectivelly
8) A "Capture the flag" match produces a line with data "red:<score> blue:<score>", indicating each team's score, similar to the `score` message above
9) In addition to the `score` events from (8), an `Exit` event is issued when the match ends "according to plan", where its DATA states the limit that was reached.

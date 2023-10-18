Contains business entities, such as log and report models.

Quake 3 log file analysis:
1) Each log line follows a well defined structure: TIME <space> EVENT_NAME: DATA
2) Comment lines are in a special form: TIME <space> <several dashes>
3) TIME is in the form HH:MM, where HH is in the 24 hour format
4) DATA has 3 forms: a map: 'name1\val1\name2\val2\...', a scalar value or custom contents
5) Each log file may contain several matches in it, enclosed in the `InitGame` and `ShutdownGame` events
6) Each match may be either in "Capture the flag" or "Deathmatch" mode
7) A "Deathmatch" has its `InitGame` line in the form: 1:47 InitGame: \sv_floodProtect\1\sv_maxPing\0\sv_minPing\0\sv_maxRate\10000\sv_minRate\0\sv_hostname\Code Miner Server\g_gametype\0\sv_privateClients\2\sv_maxclients\16\sv_allowDownload\0\bot_minplayers\0\dmflags\0\fraglimit\20\timelimit\15\g_maxGameClients\0\capturelimit\8\version\ioq3 1.36 linux-x86_64 Apr 12 2009\protocol\68\mapname\q3dm17\gamename\baseq3\g_needpass\0
8) A "Capture the flag" has its `InitGame` line in the form: 3:32 InitGame: \capturelimit\8\g_maxGameClients\0\timelimit\15\fraglimit\20\dmflags\0\bot_minplayers\0\sv_allowDownload\0\sv_maxclients\16\sv_privateClients\2\g_gametype\0\sv_hostname\Code Miner Server\sv_minRate\0\sv_maxRate\10000\sv_minPing\0\sv_maxPing\0\sv_floodProtect\1\version\ioq3 1.36 linux-x86_64 Apr 12 2009\protocol\68\mapname\q3dm17\gamename\baseq3\g_needpass\0
9) A match may end "according to plan" or due to other reasons. In the former, the game server outputs log lines with the `score` of each player before the `ShutdownGame` event; in the latter, those `score` log lines are not produced (indicating the the match didn't end due to reaching any predefined limits, such as the "fraglimit" or "capturelimit" -- for the "Deathmatch" and "Capture the flag" modes, respectivelly
10) In addition to the `score` events from (8), an `Exit` event is issued when the match ends "according to plan", where its DATA states the limit that was reached.

# Implementation Note:

This crate is very simple and, usually, creating such a small library wouldn't make sense.

Nonetheless, we are keeping the business entities isolated in their own crate to demonstrate the Multi-Tier architectural pattern used in this project,
where the `model` crate contains data (that are central to the problem being solved) that could be transferred across layers -- such as BLL, DAL and UI.
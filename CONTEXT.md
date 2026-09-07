# 274bot host

The host coordinates independent game clients, scripts, and operator views.

## Language

**Script observation**:
The game facts made available to a script for one invocation or continuation.
_Avoid_: frame, rendered frame

**Client iteration**:
One opportunity for a game client to process its ongoing simulation and pending work.
_Avoid_: server tick, presented frame

**Server tick**:
A game-time boundary established by the server, distinct from client processing and display updates.
_Avoid_: client iteration, frame

**Client fence**:
The maintained boundary between the game client and the host's bot capabilities.
_Avoid_: bot runtime inside the client

**Script compatibility surface**:
The operations, values, and lifecycle semantics supported for scripts by the host adapter.
_Avoid_: reproduction of the foreign runtime

# Submarine Returns

Parses the SQLite DB from [Submarine Tracker](https://github.com/Infiziert90/SubmarineTracker) and:

- If invoked normally on the command line, spits out submarine return times
- If running with `--daemon`:
    - Schedules a desktop notification on the local machine for each submarine's return time
    - Schedules a push notification with the [Pushover Bridge](https://github.com/tyrone-sudeium/pushover-bridge/) specified at compile time
    - Automatically watches the SQLite DB for changes and reschedules the above when it changes

## Building

`PUSHOVER_BRIDGE_URL` and `PUSHOVER_BRIDGE_PSK` are required in the environment when building:

    PUSHOVER_BRIDGE_URL="http://[server].[tailnet].ts.net:1414/message_queue.json" PUSHOVER_BRIDGE_PSK="[same psk you gave the bridge]" cargo build --release

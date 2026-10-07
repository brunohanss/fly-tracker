# Virtual aiming delay

All replay sources use a 500 ms virtual aiming delay by default. Existing configuration files also receive this default. Physical output stays disabled.

Set `processing.virtual_aim_delay_us` to change the delay. The default is `500000`. Valid values are 0 through 10000000 microseconds. Use 0 only when an immediate control regression is required.

After target confirmation, the pipeline holds one pending target ID. It continues tracking and safety evaluation on every frame. The dashboard shows the target and the remaining wait. When the wait ends, the pipeline uses the current target position and current safety evidence to request a virtual aim. It then selects the next confirmed target. The existing dwell setting can extend a target hold beyond the delay.

A safety lockout cancels the wait. Clear recovery starts a new full wait. Target loss, replay reset, shutdown, and end of input also cancel pending work. An old clear result cannot complete a pending request.

The delay uses frame source timestamps. Headless replay can run faster than real time. Paused replay does not advance the wait. Completion occurs on the first eligible frame at or after the deadline. There is no blocking sleep and no command between frames. Short sequences can finish before an aim becomes due.

This simulates a preparation delay. It does not simulate hardware movement or confirm laser position. The aim point is calculated again at completion; the predictor does not extrapolate 500 ms ahead. Processing latency reports measure CPU processing time and exclude the simulated source-time wait.

## Automated validation

`cargo test --workspace` covers the default, deadline boundaries, moving targets, target rotation, target loss, seek, shutdown, and cancellation for each lockout type. Disk replay tests apply the default delay to dog, human, and hand entry, exit, and recovery sequences. Existing immediate safety-control tests explicitly set the delay to zero. Fixture labels validate control behavior; they do not prove image detector accuracy.

The 200 ms transition fixtures have only five initial clear frames. With the default delay, the initial confirmed target cannot complete before the hazard arrives. Entry issues zero commands. Exit and recovery each issue one command after a fresh 500 ms wait.

## Host measurement

On 2026-10-07, the release pipeline benchmark processed 2000 synthetic frames per mode with the default delay. Headless processing latency was P50 510.3 us, P95 659.1 us, P99 904.7 us, and maximum 1191.7 us for the retained 1024-sample window. No input frames were dropped. This is a Windows host measurement with fixture safety. It does not measure Raspberry Pi performance or physical aiming time. The new delay state stores one target ID and one deadline; it creates no frame queue. Process memory was not measured in this run.

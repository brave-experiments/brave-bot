# Credit

Ali Shahin Shamsabadi, Brian R. Bondy, and Brendan Eich developed the idea behind this
project: that indirect prompt injection can be made structurally impossible rather than merely
unlikely, by enforcing information-flow labels at every boundary and separating routing from
content so untrusted text cannot redirect an action.

[SafeHouse](https://github.com/brave-experiments/safehouse) is the research driver and proof of
concept behind the product, brave-user-agent, this repository.

The model backend is [brave/aichat](https://github.com/brave/aichat). The client-side handling
it builds on comes from [brave/brave-core](https://github.com/brave/brave-core). The dockerized
reproducible build setup is from [bbondy/guardrails](https://github.com/bbondy/guardrails).

# Move mode visual evidence

This archive records the completed Move workflow visual comparison. The
startup screen is retained separately from the nine settled workflow
checkpoints. JSON and ANSI files are reproducibly gzip-compressed; PNG files
remain uncompressed for inspection. `SHA256SUMS` covers every archived file.

Measured result:

* 9 settled checkpoints: zero pixel differences and zero cell differences.
* Startup screen: 50,794 differing pixels and 1,159 differing cells.
* Cleanup: zero remaining PIDs on both sides.
* `full_parity`: false because the startup-screen difference remains.

The Move workflow result is evidence for this captured run and does not claim
full parity.

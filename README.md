# runstrate
Fast way to test a Substrate Runtime via RPC (eg. PolkadotJS UI).

## Build & Run

```
git clone https://github.com/arturgontijo/runstrate
cd runstrate

cargo b -r

./target/release/runstrate --help
Usage: runstrate [OPTIONS]

Options:
      --block-time <BLOCK_TIME>  milliseconds per block (0 = instant) [default: 6000]
  -h, --help                     Print help
```

Go to [PolkadotJS UI Explorer](https://polkadot.js.org/apps/?rpc=ws://127.0.0.1:9944#/explorer).

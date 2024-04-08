# Setting up MWC-wallet



## Building your own binary

### Requirements
1. All the [current requirements](https://github.com/mimblewimble/grin/blob/master/doc/build.md#requirements) of Grin.
1. [OpenSSL](https://www.openssl.org).
   * macOS with Homebrew:
      ```
      $ brew install openssl # you need to install version 1.1 of openssl for version 1.0.1 or newer of wallet713
      ``` 
   * Linux:
      ```
      $ sudo apt-get install openssl
      ```

### Installation

```
$ git clone https://github.com/whyproject/why-wallet.git
$ cd why-wallet
$ cargo build --release
```
MWC-wallet needs to run against a node, you can connect to a local node and a remote node. 
For details about how to install a local node, please refer to the page:[Grin's Wiki](https://github.com/mimblewimble/docs/wiki/Wallet-User-Guide)

The following steps is to show how to run against a remote node. 
MWC-wallet needs be be initiated first.
```
$ cd target/release
$ ./why-wallet init [flags]
```

If you'd like to run against floonet, use:
```
$ cd target/release
$ ./why-wallet --floonet init [flags]
```
--help will help to list all the available flags
```
$ ./why-wallet --floonet init --help
```

After wallet is initiated, why-wallet.toml file will be generated( either in the default ~/.why directory or current directory )
Open this file, update the parameter check_node_api_http_addr to the address of the remote node.
The address can contain multiple nodes for failover purpose. There needs be a semicolon in between the addresses.
For example: https://why713.floonet.why.mw;https://why7132.floonet.why.mw;https://why7133.floonet.why.mw;
https://why7134.floonet.why.mw

Mainnet: why713.why.mw why71362.why.mw why7133.why.mw why7134.why.mw why7135.why.mw why7136.why.mw
Floonet: why713.floonet.why.mw why7132.floonet.why.mw why7133.floonet.why.mw why7134.floonet.why.mw


api_seed in the .api_seed file(same directory as why-wallet.toml file) will also be updated.


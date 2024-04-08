# Overview #

As of why-wallet 3.2.2, tor is supported externally from why-wallet. In the previous versions, the tor support only allowed
for tor to be executed internally by why-wallet. There were significant performance impacts to that model. This feature will
allow for real production use of tor for withdrawals by both exchanges and mining pools.

# Configuration #

The configuration of tor is the same as in previous versions with the exception of a new parameter in the why-wallet.toml
[tor] section, called socks_running. By default it is set to false. If you wish to use an external tor socks proxy, you will
need to set this parameter to true. The tor section of your why-wallet.toml may look like this:

```
[tor]
# Whether to start tor listener on listener startup (default true)
use_tor_listener = true

# TOR (SOCKS) proxy server address
socks_proxy_addr = "127.0.0.1:59050"

#Directory to output TOR configuration to when sending
send_config_dir = "/Users/test/.why/main"
socks_running = false
```
Note: use socks_running true if you already have a tor sock process setup. To get much better performance it is 
recommended to have sock process allways running. Otherwise wallet will run it for every tor interaction.

This configuration tells the why-wallet not to instantiate a tor instance each time send is called and instead the why-wallet
will expect that a tor instance is already running. You are expected to have a tor instance running.

To install/configure tor, you can use the following command on linux:

```# sudo apt install tor```

Or on macos:

```# brew install tor```

To configure tor, your torrc might look like this:

```
SocksPort 127.0.0.1:59050
DataDirectory ./data
```

Note that the "data" directory should exist at the same level as your torrc file and the SocksPort variable should be the same
value used in the why-wallet.toml.

To start tor, run this command:

```# tor -f /path/to/torrc```

Then sends will use this existing tor instance instead of instantiating a new one each time. The performance will be much
better.

# Migration information #

Since previous versions of why-wallet did not have this socks_running parameter, if you are upgrading to this version of the
wallet, you will need to add this parameter to the config file or generate a new why-wallet.toml with the why-wallet init
command.

# Conclusion #

Instantiating a new tor instance every time is too slow and this feature allows for an externally running tor instance to be
used with why-wallet. This is a feature that hopefully all exchanges and pools will support. The latest version of the
Qt wallet makes it easy for users to accept payments via tor address and it is more secure and private for exchanges and pools
to enable this withdrawal method.

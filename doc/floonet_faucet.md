# MWC Floonet Faucet / MWC Testnet Faucet

If you neeed some coins to test with MWC floonet, please be free to use the faucet that listen on MWC MQS address `xmgEvZ4MCCGMJnRnNXKHBbHmSGWQchNr9uZpY5J1XXnsCFS45fsU`.

Faucet is running is why713 wallet, so we recommend to use why713 wallet to request the coins. Then you can transafer them to your QT wallet or why-wallet. 

You can download why713 from here: https://github.com/whyproject/why713/releases 

We are assuming that you already download, installed and provision your why713 wallet. Here are how you can request the coins. Please note, you can request maximun 5 MWC at a time.

### How to request the coins

```
> why713 --floonet
Using wallet configuration file at ......

Welcome to wallet713 for MWC v4.1.0

Unlock your existing wallet or type 'init' to initiate a new one
Use 'help' to see available commands

ERROR: The wallet is locked. Please use 'unlock' first.
wallet713>
wallet713> unlock -p XXXXXXXXXX
Your whymqs address: xmj6hXXXXXXXXXXXXX
wallet713>
wallet713> listen -s
Starting whymqs listener...
wallet713>
whymqs listener started for [xmj6hTX7UKAXXXXXXXXXXXXXX] tid=[kbxsjQ2TAo0jjLsl8Ib_L]

wallet713> invoice 1.5 --to xmgEvZ4MCCGMJnRnNXKHBbHmSGWQchNr9uZpY5J1XXnsCFS45fsU
slate [c7831053-80fb-4956-8abd-f2b270afc5ff] for [1.500000000] MWCs sent to [whymqs://xmgEvZ4MCCGMJnRnNXKHBbHmSGWQchNr9uZpY5J1XXnsCFS45fsU]
slate [c7831053-80fb-4956-8abd-f2b270afc5ff] received back from [xmgEvZ4MCCGMJnRnNXKHBbHmSGWQchNr9uZpY5J1XXnsCFS45fsU] for [1.500000000] MWCs
```

Please note, if faucet not used for a long time, it might take few minutes to wakeup and resync with a blockchain. If you invoice failed,
please wait for 10 minutes and try again. If it is still offline, please ping any moderator at Discord( https://discord.gg/n5dZaty ) 'developers' channel.  

### How to return the coin 

When you finish with your tests, please send the coins back to faucet. 
```
send 3.123 --to xmgEvZ4MCCGMJnRnNXKHBbHmSGWQchNr9uZpY5J1XXnsCFS45fsU -c 1
```
# Eld Node App

This directory is a **parallel copy** of `eld/chain/node_app`. Deploy, Docker, and local orchestration in the sibling `eld` repo still run that original crate. Wallets, P2P keypairs, and runtime `data/` are not shipped here.





## Sync with Tendermint node data

The Eld node app ("Eld") and the Tendermint Node ("TM") may have saved block data after a shutdown. Depending on the state of the data, a simple restart may work, or the app data may need to be wiped. Below is a list of the cases: 

### Cases

#### Case 1: Both Eld and TM are clean

Blockchain will start at node 0

#### Case 2: Eld is clean, TM has saved data

TM will automatically replay saved block data up until its latest block, then continue block processing

It's important Eld's logic in the abci callbacks is completely deterministic. For example, we use BTreeMap instead of HashMap for tries, as HashMap's key ordering isn't deterministic (apparently so, discovered in testing). Determinism in the abci callbacks will allow the same app hash to be produced at the same height, which is a requirement for TM to keep running.

#### Case 3: Eld has saved data and TM has saved data, and TM height >= Eld height

As long as the height in TM is >= Eld block height, TM will start at height and produce blocks for Eld to process. Normally on shutdown of Eld app, the two stored heights will be the same.

#### Case 4: Both have saved data, and TM height < Eld height

Unrecoverable at runtime.
Need to clean the Eld saved data and start again..

### Run Eld locally

Start Eld with data: ./start_with_db_data.sh
Start Eld without data: ./start_app_clean.sh

Start TM with data: tendermint node --proxy_app=tcp://127.0.0.1:26658 --log_level debug
Start TM without data: tendermint unsafe_reset_all && tendermint node --proxy_app=tcp://127.0.0.1:26658 --log_level debug

Example tx, in cli: cargo run transfer wallet1 0x23b1f0b6199479b5d04fb54e21df14d51530b7b1 59
View account: cargo run get-account 0xe17404c417fa10cc04fdf73604fcacca8d0a687c 

### Run Eld on Docker Compose

#### Multi node

docker-compose -f deploy/docker/docker-compose-multi.yaml up --build

#### Single node

docker-compose -f deploy/docker/docker-compose-single.yaml up --build

### How does TM retrieve the Eld state

Via the Info.info() method which includes app hash and height from the (restored) Eld state

### How can we find out the TM block height and app hash

Except looking at the latest block data, you can also call TM rpc endpoint status()
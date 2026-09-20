Tendermint's RPC API does support submitting transactions via POST requests, which is useful for handling large transaction payloads like bytecode for contract code. The issue you're encountering with the "header too large" error likely stems from the size of the transaction data when using the /broadcast_tx_commit endpoint with a GET request or when the HTTP headers exceed the server's configured limits due to large query parameters. Here's how you can address this using Tendermint's RPC API with POST requests and handle large transactions effectively.
Solution: Use JSON-RPC over HTTP POST
Tendermint provides a JSON-RPC interface that allows you to submit transactions via HTTP POST, which supports larger payloads in the request body, avoiding header size limitations. The relevant JSON-RPC method for broadcasting transactions is broadcast_tx_commit, but you can also use broadcast_tx_sync or broadcast_tx_async depending on your needs for confirmation guarantees.
Steps to Submit a Large Transaction
Use the JSON-RPC Endpoint:
The JSON-RPC endpoint is typically available at the root RPC endpoint, e.g., http://<node>:26657/.
Instead of using the REST-like /broadcast_tx_commit?tx=... endpoint with query parameters, send a POST request to the root RPC endpoint with a JSON-RPC payload.
Construct the JSON-RPC Request:
The request should include the method (broadcast_tx_commit), the transaction bytes (encoded as a string), and an ID for the request.
The transaction bytes should be base64-encoded or hex-encoded, depending on your application's formatting (Tendermint typically expects the raw transaction bytes).
Example JSON-RPC payload:
json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "broadcast_tx_commit",
  "params": ["<base64-encoded-tx-bytes>"]
}
Replace <base64-encoded-tx-bytes> with the base64-encoded representation of your transaction, which includes the large bytecode for the contract.
Send the POST Request:
Use a tool like curl or a programming library (e.g., requests in Python) to send the POST request to the Tendermint node's RPC endpoint.
Example using curl:
bash
curl --header "Content-Type: application/json" --request POST --data '{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "broadcast_tx_commit",
  "params": ["<base64-encoded-tx-bytes>"]
}' http://<node>:26657/
Replace <node> with your Tendermint node's address (e.g., localhost or the node's IP) and <base64-encoded-tx-bytes> with your transaction data.
Handle the Response:
The response will include the result of the transaction broadcast, including whether it was successfully committed to a block (for broadcast_tx_commit).
Example response for a successful broadcast:
json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "check_tx": {
      "code": 0,
      "log": "",
      "data": ""
    },
    "deliver_tx": {
      "code": 0,
      "log": "",
      "data": ""
    },
    "hash": "<tx-hash>",
    "height": "<block-height>"
  },
  "error": ""
}
If there's an error (e.g., transaction too large or invalid), the error field will contain details, or the check_tx/deliver_tx fields may indicate a non-zero code.
Example with Python
Here's a Python example using the requests library to submit a large transaction:
python
import requests
import base64

# Your transaction bytes (replace with actual transaction data)
tx_bytes = b"<your-large-bytecode-transaction-data>"
tx_base64 = base64.b64encode(tx_bytes).decode("utf-8")

# JSON-RPC payload
payload = {
    "jsonrpc": "2.0",
    "id": 1,
    "method": "broadcast_tx_commit",
    "params": [tx_base64]
}

# Send POST request
url = "http://localhost:26657/"
headers = {"Content-Type": "application/json"}
response = requests.post(url, json=payload, headers=headers)

# Check response
if response.status_code == 200:
    print(response.json())
else:
    print(f"Error: {response.status_code} - {response.text}")
Handling Large Transactions
If you still encounter issues with large transactions, consider the following:
Check Tendermint Configuration:
Tendermint has a maximum transaction size limit defined by the consensus_params.block.max_bytes in the node's configuration (default is often 22020096 bytes, as seen in some genesis configurations). Ensure your transaction size is within this limit.
If you're running your own node, you can increase max_bytes in the genesis file, but this requires consensus among validators and may not be feasible for public networks.
Verify the node's HTTP server configuration (e.g., for Nginx or Tendermint's internal server) to ensure it allows large request bodies. For example, in Nginx, you may need to set client_max_body_size to a higher value.
Split the Bytecode:
If the bytecode is too large for a single transaction, consider splitting the contract deployment into multiple transactions. For example:
Deploy the contract with an initial placeholder code.
Use subsequent transactions to append or update the bytecode via contract methods.
This approach depends on the application logic and the blockchain's application layer (e.g., Cosmos SDK or Ethermint for EVM-compatible chains).
Use broadcast_tx_sync or broadcast_tx_async:
If broadcast_tx_commit is too slow or resource-intensive for large transactions, try broadcast_tx_sync (returns after CheckTx but before block inclusion) or broadcast_tx_async (returns immediately without waiting for CheckTx results). These methods reduce the node's processing burden but provide weaker guarantees about transaction inclusion.
Example payload for broadcast_tx_sync:
json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "broadcast_tx_sync",
  "params": ["<base64-encoded-tx-bytes>"]
}
Increase Socket Buffer Size:
As noted in a GitHub issue, large transactions can fail if the node's socket buffer is too small, causing partial data reads. If you're running a custom ABCI application or Tendermint node, ensure the socket buffer is sufficiently large (e.g., increase it in your server's configuration, such as Erlang's Ranch or Go's net/http).
This is more relevant if you're developing the ABCI application and less likely to be an issue with standard Tendermint nodes.
Validate Transaction Size Locally:
Before broadcasting, validate the transaction size against the network's max_bytes limit. You can query the node's genesis or consensus parameters using the genesis or consensus_params RPC methods:
bash
curl --header "Content-Type: application/json" --request POST --data '{"jsonrpc":"2.0","method":"genesis","params":[],"id":1}' http://<node>:26657/
Check the consensus_params.block.max_bytes field in the response.
Use Websockets for Large Transactions:
Tendermint supports JSON-RPC over WebSockets, which can handle larger payloads more reliably in some cases. Connect to the WebSocket endpoint (e.g., ws://<node>:26657/websocket) and send the same JSON-RPC payload as above.
Example WebSocket subscription for transaction results is also possible to monitor inclusion.
Addressing the "Header Too Large" Error
The "header too large" error suggests that the transaction data in the query parameter (tx=...) is causing the HTTP headers to exceed the server's limit. By switching to a POST request with the transaction in the body (as shown above), you avoid this issue because the body is not subject to the same header size restrictions. If the error persists with POST requests:
Check Server Configuration:
Ensure the Tendermint node's HTTP server (or reverse proxy like Nginx) allows large request bodies. For Nginx, add:
nginx
client_max_body_size 50M;
to the server block and restart the server.
Reduce Transaction Size:
If the transaction is still too large, compress the bytecode (if supported by the application) or split it as mentioned above.
Test with a Smaller Payload:
Verify the endpoint works with a smaller transaction to rule out other configuration issues:
bash
curl --header "Content-Type: application/json" --request POST --data '{"jsonrpc":"2.0","id":1,"method":"broadcast_tx_commit","params":["dGVzdA=="]}' http://<node>:26657/
(Here, dGVzdA== is the base64 encoding of the string test.)
Additional Notes
Known Issue with Large Transactions: A 2018 GitHub issue highlighted that broadcast_tx_sync struggled with large transactions due to socket buffer issues in some ABCI implementations. While this was specific to an older Tendermint version (0.26.0), it underscores the importance of ensuring your node's socket and buffer configurations are adequate.
Cosmos SDK or Ethermint: If you're using Tendermint with Cosmos SDK or Ethermint (for EVM-compatible contracts), the transaction format and bytecode handling may depend on the application layer. Ensure the transaction is correctly formatted for the specific chain (e.g., Cosmos SDK's Tx structure or EVM's RLP-encoded transactions).
Network-Specific Limits: If you're on a specific Cosmos-based chain (e.g., Crypto.org, Osmosis), check the chain's documentation or chain registry for RPC endpoints and transaction size limits.
Example for EVM-Compatible Chains (e.g., Ethermint)
If you're deploying a smart contract on an EVM-compatible Tendermint chain like Ethermint, the transaction data should follow Ethereum's format (e.g., RLP-encoded with bytecode in the data field). Here's an example of preparing the transaction:
python
from eth_account import Account
from eth_account.messages import encode_defunct
import rlp
from rlp.sedes import Binary, big_endian_int
from web3 import Web3

# Example: Create a transaction for contract deployment
w3 = Web3(Web3.HTTPProvider("http://<node>:26657"))
account = Account.from_key("<your-private-key>")
nonce = w3.eth.get_transaction_count(account.address)
bytecode = "0x6060604052..."  # Your large contract bytecode
tx = {
    "nonce": nonce,
    "gasPrice": w3.eth.gas_price,
    "gas": 3000000,
    "to": None,  # None for contract creation
    "value": 0,
    "data": bytecode,
    "chainId": w3.eth.chain_id
}
signed_tx = account.sign_transaction(tx)
raw_tx = rlp.encode(signed_tx.rawTransaction)
tx_base64 = base64.b64encode(raw_tx).decode("utf-8")

# Submit via Tendermint JSON-RPC
payload = {
    "jsonrpc": "2.0",
    "id": 1,
    "method": "broadcast_tx_commit",
    "params": [tx_base64]
}
response = requests.post("http://<node>:26657/", json=payload, headers={"Content-Type": "application/json"})
print(response.json())
This example assumes an EVM-compatible chain; adjust the transaction format for Cosmos SDK-based chains if needed.
Conclusion
To handle large transactions in Tendermint, use the JSON-RPC broadcast_tx_commit method over HTTP POST, sending the base64-encoded transaction in the request body. This avoids header size limits and supports larger payloads. If issues persist, verify the node's max_bytes limit, increase server body size limits, or split the bytecode into multiple transactions. For EVM-compatible chains, ensure the transaction is correctly formatted for the EVM. Test with smaller payloads to isolate configuration issues, and consult the chain's documentation for specific limits or requirements.
If you need further assistance with formatting the transaction or debugging specific errors, please provide more details about the transaction structure, the error message, or the Tendermint-based chain you're using (e.g., Cosmos, Ethermint).
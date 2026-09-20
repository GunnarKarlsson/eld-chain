// Capacity validators query:

curl -s -X POST http://localhost:26657 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": -1,
    "method": "abci_query",
    "params": {
      "path": "capacity_validators",
      "data": "",
      "prove": false
    }
  }'

// Example response shape (fields vary with chain state):
{
  "jsonrpc": "2.0",
  "id": -1,
  "result": {
    "response": {
      "code": 0,
      "log": "Capacity validators retrieved",
      "info": "{\"capacity_validators\":[...],\"total_stake\":0,\"total_capacity\":52428800,\"current_epoch\":15}"
    }
  }
}

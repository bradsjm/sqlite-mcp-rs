#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

SQLITE_PERSIST_ROOT="${SQLITE_PERSIST_ROOT:-$REPO_ROOT/.tmp/sqlite-mcp-inspector}"
SQLITE_LOG_LEVEL="${SQLITE_LOG_LEVEL:-info}"
SQLITE_MAX_SQL_LENGTH="${SQLITE_MAX_SQL_LENGTH:-20000}"
SQLITE_MAX_STATEMENTS="${SQLITE_MAX_STATEMENTS:-50}"
SQLITE_MAX_ROWS="${SQLITE_MAX_ROWS:-500}"
SQLITE_MAX_BYTES="${SQLITE_MAX_BYTES:-1048576}"
SQLITE_MAX_DB_BYTES="${SQLITE_MAX_DB_BYTES:-100000000}"
SQLITE_CURSOR_TTL_SECONDS="${SQLITE_CURSOR_TTL_SECONDS:-600}"
SQLITE_CURSOR_CAPACITY="${SQLITE_CURSOR_CAPACITY:-500}"

SQLITE_INSPECTOR_DB_PATH="${SQLITE_INSPECTOR_DB_PATH:-inspector/seed.db}"
SQLITE_INSPECTOR_SEED_SQL="${SQLITE_INSPECTOR_SEED_SQL:-$REPO_ROOT/tests/fixtures/sqlite-inspector-seed.sql}"
SQLITE_RERANKER_PROVIDER="${SQLITE_RERANKER_PROVIDER:-fastembed}"
SQLITE_RERANKER_MODEL="${SQLITE_RERANKER_MODEL:-BAAI/bge-reranker-base}"
SQLITE_MODEL_CACHE_DIR="${SQLITE_MODEL_CACHE_DIR:-$REPO_ROOT/.inspector-cache/huggingface}"

if [[ "$#" -eq 0 ]]; then
  echo "usage: scripts/test-sqlite-mcp-inspector.sh <mcp-server-command> [args...]" >&2
  exit 2
fi

SERVER_COMMAND=("$@")

while [[ "${#SERVER_COMMAND[@]}" -gt 0 ]]; do
  last_index=$((${#SERVER_COMMAND[@]} - 1))
  if [[ "${SERVER_COMMAND[$last_index]}" != "--" ]]; then
    break
  fi
  unset 'SERVER_COMMAND[$last_index]'
done

if [[ "${#SERVER_COMMAND[@]}" -eq 0 ]]; then
  echo "server command cannot be empty" >&2
  exit 2
fi

probe_persist_root() {
  python3 - "$SQLITE_PERSIST_ROOT" <<'PY'
import os
import sys

root = sys.argv[1]
probe_path = os.path.join(root, ".write-probe")

try:
    os.makedirs(root, exist_ok=True)
    with open(probe_path, "w", encoding="utf-8") as handle:
        handle.write("ok")
    os.remove(probe_path)
except Exception as exc:
    print(exc)
    sys.exit(1)
PY
}

wait_for_persist_root() {
  local attempts=10
  local last_probe_error=""

  for _ in $(seq 1 "$attempts"); do
    if last_probe_error=$(probe_persist_root 2>&1); then
      return 0
    fi
    sleep 1
  done

  echo "persist root unavailable at ${SQLITE_PERSIST_ROOT} after ${attempts}s: ${last_probe_error}" >&2
  return 1
}

seed_sqlite_db() {
  python3 - "$SQLITE_PERSIST_ROOT" "$SQLITE_INSPECTOR_DB_PATH" "$SQLITE_INSPECTOR_SEED_SQL" <<'PY'
import os
import sqlite3
import sys

root = os.path.realpath(sys.argv[1])
relative_path = sys.argv[2]
seed_sql = sys.argv[3]

candidate = os.path.realpath(os.path.join(root, relative_path))
if os.path.commonpath([root, candidate]) != root:
    print("SQLITE_INSPECTOR_DB_PATH must remain inside SQLITE_PERSIST_ROOT")
    sys.exit(1)

os.makedirs(os.path.dirname(candidate), exist_ok=True)
connection = sqlite3.connect(candidate)

try:
    if os.path.isfile(seed_sql):
        with open(seed_sql, "r", encoding="utf-8") as handle:
            connection.executescript(handle.read())
    else:
        connection.execute("PRAGMA journal_mode=WAL;")
        connection.execute(
            "CREATE TABLE IF NOT EXISTS healthcheck (id INTEGER PRIMARY KEY, label TEXT NOT NULL);"
        )
        connection.execute(
            "INSERT INTO healthcheck (label) SELECT ? WHERE NOT EXISTS (SELECT 1 FROM healthcheck);",
            ("inspector-ready",),
        )
    connection.commit()
finally:
    connection.close()
PY
}

require_tools() {
  command -v jq >/dev/null 2>&1 || {
    echo "jq is required" >&2
    exit 1
  }
  command -v npx >/dev/null 2>&1 || {
    echo "npx is required" >&2
    exit 1
  }
}

run_inspector() {
  SQLITE_PERSIST_ROOT="$SQLITE_PERSIST_ROOT" \
  SQLITE_LOG_LEVEL="$SQLITE_LOG_LEVEL" \
  SQLITE_MAX_SQL_LENGTH="$SQLITE_MAX_SQL_LENGTH" \
  SQLITE_MAX_STATEMENTS="$SQLITE_MAX_STATEMENTS" \
  SQLITE_MAX_ROWS="$SQLITE_MAX_ROWS" \
  SQLITE_MAX_BYTES="$SQLITE_MAX_BYTES" \
  SQLITE_MAX_DB_BYTES="$SQLITE_MAX_DB_BYTES" \
  SQLITE_CURSOR_TTL_SECONDS="$SQLITE_CURSOR_TTL_SECONDS" \
  SQLITE_CURSOR_CAPACITY="$SQLITE_CURSOR_CAPACITY" \
  SQLITE_INSPECTOR_DB_PATH="$SQLITE_INSPECTOR_DB_PATH" \
  SQLITE_RERANKER_PROVIDER="$SQLITE_RERANKER_PROVIDER" \
  SQLITE_RERANKER_MODEL="$SQLITE_RERANKER_MODEL" \
  SQLITE_EMBEDDING_CACHE_DIR="$SQLITE_MODEL_CACHE_DIR" \
  SQLITE_RERANKER_CACHE_DIR="$SQLITE_MODEL_CACHE_DIR" \
  npx -y @modelcontextprotocol/inspector --cli "${SERVER_COMMAND[@]}" "$@"
}

assert_json() {
  local json="$1"
  shift

  printf '%s\n' "$json" | jq -e "$@" >/dev/null
}

expect_failure_with_text() {
  local expected_text="$1"
  shift
  local output

  if output=$(run_inspector "$@" 2>&1); then
    echo "Expected inspector call to fail, but it succeeded" >&2
    exit 1
  fi

  if [[ "$output" != *"$expected_text"* ]]; then
    echo "Failure did not include expected text: $expected_text" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
}

run_tests() {
  local active_db_id
  local tools_json
  local initial_list_json
  local open_json
  local list_json
  local query_json
  local execute_json
  local batch_json
  local create_import_table_json
  local import_json
  local vector_status_json
  local vector_create_json
  local vector_upsert_json
  local vector_list_json
  local vector_search_json
  local vector_rerank_json
  local has_vector_tools
  local vector_status_err
  local vector_run_id

  echo "Checking MCP tool discovery"
  tools_json=$(run_inspector --method tools/list)
  assert_json "$tools_json" '
    ((.tools // .result.tools // []) | map(.name)) as $names
    | ($names | index("db_open") != null)
    and ($names | index("db_list") != null)
    and ($names | index("sql_query") != null)
    and ($names | index("sql_execute") != null)
    and ($names | index("sql_batch") != null)
    and ($names | index("db_import") != null)
  '

  has_vector_tools=$(printf '%s\n' "$tools_json" | jq -r '
    ((.tools // .result.tools // []) | map(.name)) as $names
    | if (($names | index("vector_status") != null)
      and ($names | index("vector_collection_create") != null)
      and ($names | index("vector_upsert") != null)
      and ($names | index("vector_search") != null)
      and ($names | index("vector_collection_list") != null))
      then "true" else "false" end
  ')
  vector_run_id=$(date +%s)

  echo "Checking default database bootstrap through MCP inspector"
  initial_list_json=$(run_inspector --method tools/call --tool-name db_list)
  assert_json "$initial_list_json" '
    (.isError != true)
    and ((.structuredContent.data // .data // .result.structuredContent.data // .result.data).active_db_id == "default")
    and (((.structuredContent.data // .data // .result.structuredContent.data // .result.data).open // []) | map(.db_id) | index("default") != null)
  '

  echo "Opening persisted DB through MCP inspector"
  open_json=$(run_inspector \
    --method tools/call \
    --tool-name db_open \
    --tool-arg mode=persist \
    --tool-arg "path=${SQLITE_INSPECTOR_DB_PATH}" \
    --tool-arg reset=true)
  assert_json "$open_json" '
    (.isError != true)
    and (((.structuredContent.data // .data // .result.structuredContent.data // .result.data).db_id // "") != "")
    and ((.structuredContent.data // .data // .result.structuredContent.data // .result.data).active == true)
    and ((.structuredContent.data // .data // .result.structuredContent.data // .result.data).mode == "persist")
  '
  active_db_id=$(printf '%s\n' "$open_json" | jq -r '(.structuredContent.data // .data // .result.structuredContent.data // .result.data).db_id // empty')
  if [[ -z "$active_db_id" ]]; then
    echo "db_open did not return db_id" >&2
    exit 1
  fi

  echo "Checking db_list through MCP inspector"
  list_json=$(run_inspector --method tools/call --tool-name db_list)
  assert_json "$list_json" --arg db_id "$active_db_id" '
    (.isError != true)
    and ((.structuredContent.data // .data // .result.structuredContent.data // .result.data).active_db_id == $db_id)
    and (((.structuredContent.data // .data // .result.structuredContent.data // .result.data).open // []) | map(.db_id) | index($db_id) != null)
  '

  echo "Running sql_query through MCP inspector"
  query_json=$(run_inspector \
    --method tools/call \
    --tool-name sql_query \
    --tool-arg "db_id=${active_db_id}" \
    --tool-arg "sql=SELECT COUNT(*) AS cnt FROM sample_items;")
  assert_json "$query_json" '
    (.isError != true)
    and (((.structuredContent.data // .data // .result.structuredContent.data // .result.data).rows[0].cnt // 0) >= 1)
  '

  echo "Running sql_execute through MCP inspector"
  execute_json=$(run_inspector \
    --method tools/call \
    --tool-name sql_execute \
    --tool-arg "db_id=${active_db_id}" \
    --tool-arg "sql=INSERT INTO sample_items(name, qty) VALUES ('delta', 13);")
  assert_json "$execute_json" '
    (.isError != true)
    and ((.structuredContent.data // .data // .result.structuredContent.data // .result.data).rows_affected == 1)
  '

  echo "Running sql_batch through MCP inspector"
  batch_json=$(run_inspector \
    --method tools/call \
    --tool-name sql_batch \
    --tool-arg "db_id=${active_db_id}" \
    --tool-arg transaction=required \
    --tool-arg "statements=[{\"sql\":\"UPDATE sample_items SET qty = qty + 1 WHERE name = 'alpha';\"},{\"sql\":\"DELETE FROM sample_items WHERE name = 'delta';\"}]")
  assert_json "$batch_json" '
    (.isError != true)
    and ((.structuredContent.data // .data // .result.structuredContent.data // .result.data).executed == 2)
  '

  echo "Creating import table with sql_execute"
  create_import_table_json=$(run_inspector \
    --method tools/call \
    --tool-name sql_execute \
    --tool-arg "db_id=${active_db_id}" \
    --tool-arg "sql=CREATE TABLE IF NOT EXISTS imported_items (name TEXT NOT NULL, qty INTEGER NOT NULL);")
  assert_json "$create_import_table_json" '(.isError != true)'

  echo "Running db_import through MCP inspector"
  import_json=$(run_inspector \
    --method tools/call \
    --tool-name db_import \
    --tool-arg "db_id=${active_db_id}" \
    --tool-arg format=json \
    --tool-arg table=imported_items \
    --tool-arg 'data=[{"name":"from_json_a","qty":5},{"name":"from_json_b","qty":8}]')
  assert_json "$import_json" '
    (.isError != true)
    and ((.structuredContent.data // .data // .result.structuredContent.data // .result.data).rows_inserted == 2)
  '

  if [[ "$has_vector_tools" == "true" ]]; then
    echo "Pre-downloading vector and reranker models"
    HF_CACHE_DIR="$SQLITE_MODEL_CACHE_DIR" bash "$REPO_ROOT/scripts/download-models.sh"

    echo "Running vector_status through MCP inspector"
    vector_status_err=$(mktemp)
    if ! vector_status_json=$(run_inspector \
      --method tools/call \
      --tool-name vector_status \
      --tool-arg "db_id=${active_db_id}" \
      --tool-arg prewarm=true 2>"$vector_status_err"); then
      vector_status_json=$(cat "$vector_status_err")
      if [[ "$vector_status_json" == *"vector feature is not enabled"* ]]; then
        echo "Vector tools are listed but vector feature is disabled at runtime; skipping vector/embedding/reranking inspector checks"
        vector_status_json=""
      else
        echo "$vector_status_json" >&2
        rm -f "$vector_status_err"
        exit 1
      fi
    fi
    rm -f "$vector_status_err"

    if [[ -n "$vector_status_json" ]]; then
    assert_json "$vector_status_json" '
      (.isError != true)
      and ((.structuredContent.data // .data // .result.structuredContent.data // .result.data).embedding.ready == true)
      and (((.structuredContent.data // .data // .result.structuredContent.data // .result.data).reranker // {"ready": false}).ready == true)
    '

    echo "Creating vector collection through MCP inspector"
    vector_create_json=$(run_inspector \
      --method tools/call \
      --tool-name vector_collection_create \
      --tool-arg "db_id=${active_db_id}" \
      --tool-arg collection=inspector_vectors \
      --tool-arg if_not_exists=true)
    assert_json "$vector_create_json" '(.isError != true)'

    echo "Upserting vector documents through MCP inspector"
    vector_upsert_json=$(run_inspector \
      --method tools/call \
      --tool-name vector_upsert \
      --tool-arg "db_id=${active_db_id}" \
      --tool-arg collection=inspector_vectors \
      --tool-arg on_conflict=replace \
      --tool-arg "items=[{\"id\":\"${vector_run_id}_doc_a\",\"text\":\"SQLite stores structured records in local files.\",\"metadata\":{\"topic\":\"db\"}},{\"id\":\"${vector_run_id}_doc_b\",\"text\":\"Embeddings map text into vector space.\",\"metadata\":{\"topic\":\"ml\"}},{\"id\":\"${vector_run_id}_doc_c\",\"text\":\"Reranking improves final retrieval quality.\",\"metadata\":{\"topic\":\"search\"}}]")
    assert_json "$vector_upsert_json" '
      (.isError != true)
      and ((.structuredContent.data // .data // .result.structuredContent.data // .result.data).upserted_count == 3)
    '

    echo "Listing vector collections through MCP inspector"
    vector_list_json=$(run_inspector \
      --method tools/call \
      --tool-name vector_collection_list \
      --tool-arg "db_id=${active_db_id}")
    assert_json "$vector_list_json" '
      (.isError != true)
      and (((.structuredContent.data // .data // .result.structuredContent.data // .result.data).collections // [])
        | map(select(.collection == "inspector_vectors"))
        | length) == 1
    '

    echo "Searching vectors with embedding similarity through MCP inspector"
    vector_search_json=$(run_inspector \
      --method tools/call \
      --tool-name vector_search \
      --tool-arg "db_id=${active_db_id}" \
      --tool-arg collection=inspector_vectors \
      --tool-arg 'query_text=How do embeddings represent text?' \
      --tool-arg top_k=2 \
      --tool-arg include_text=true)
    assert_json "$vector_search_json" '
      (.isError != true)
      and ((.structuredContent.data // .data // .result.structuredContent.data // .result.data).matches | length) >= 1
    '

    echo "Searching vectors with reranking through MCP inspector"
    vector_rerank_json=$(run_inspector \
      --method tools/call \
      --tool-name vector_search \
      --tool-arg "db_id=${active_db_id}" \
      --tool-arg collection=inspector_vectors \
      --tool-arg 'query_text=Which item talks about reranking?' \
      --tool-arg top_k=2 \
      --tool-arg rerank=on \
      --tool-arg rerank_fetch_k=3 \
      --tool-arg include_text=true)
    assert_json "$vector_rerank_json" '
      (.isError != true)
      and ((.structuredContent.data // .data // .result.structuredContent.data // .result.data).reranked == true)
      and ((.structuredContent.data // .data // .result.structuredContent.data // .result.data).matches | length) >= 1
    '
    fi
  else
    echo "Vector tools not available in this build; skipping vector/embedding/reranking inspector checks"
  fi

  echo "Checking destructive guard over MCP"
  expect_failure_with_text "confirm_destructive=true" \
    --method tools/call \
    --tool-name sql_batch \
    --tool-arg "db_id=${active_db_id}" \
    --tool-arg transaction=required \
    --tool-arg "statements=[{\"sql\":\"DELETE FROM sample_items;\"}]"

  echo "MCP inspector SQLite integration checks passed"
}

wait_for_persist_root
seed_sqlite_db
require_tools
run_tests

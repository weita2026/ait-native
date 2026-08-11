#!/bin/sh

set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <perfetto-trace>" >&2
    exit 2
fi

trace_path=$1
if [ ! -f "$trace_path" ]; then
    echo "Perfetto trace does not exist: $trace_path" >&2
    exit 2
fi

trace_processor=${TRACE_PROCESSOR_SHELL:-}
if [ -z "$trace_processor" ]; then
    trace_processor=$(command -v trace_processor_shell || true)
fi
if [ -z "$trace_processor" ] || [ ! -x "$trace_processor" ]; then
    echo "trace_processor_shell is required; set TRACE_PROCESSOR_SHELL explicitly" >&2
    exit 2
fi

minimum_samples=${AIT_ATOMIC_WORKFLOW_PERF_MIN_SAMPLES:-30}
# Default release p95 budgets for the bounded ten-entry workload. Operators
# may override them to qualify a different hardware class explicitly.
history_prepare_budget_ms=${AIT_HISTORY_PROMOTION_PREPARE_P95_MS:-75}
history_writer_budget_ms=${AIT_HISTORY_PROMOTION_WRITER_P95_MS:-70}
task_land_budget_ms=${AIT_ATOMIC_TASK_LAND_P95_MS:-150}
task_land_writer_budget_ms=${AIT_ATOMIC_TASK_LAND_WRITER_P95_MS:-100}

query_one() {
    "$trace_processor" query "$trace_path" "$1" 2>/dev/null | tail -n 1
}

trace_errors=$(query_one "
    select coalesce(sum(value), 0)
    from stats
    where severity = 'error';
")
if [ "$trace_errors" -ne 0 ]; then
    echo "Perfetto reported $trace_errors trace-health errors" >&2
    exit 1
fi
echo "Perfetto trace-health errors=0"

check_range() {
    range_name=$1
    budget_ms=$2
    result=$(query_one "
        select
          count(*) as samples,
          round(percentile(dur / 1000000.0, 0.50), 3) as p50_ms,
          round(percentile(dur / 1000000.0, 0.95), 3) as p95_ms
        from slice
        where name = '$range_name';
    ")
    old_ifs=$IFS
    IFS=,
    set -- $result
    IFS=$old_ifs
    samples=${1:-0}
    p50_ms=${2:-}
    p95_ms=${3:-}
    if [ "$samples" -lt "$minimum_samples" ]; then
        echo "$range_name has $samples samples; expected at least $minimum_samples" >&2
        exit 1
    fi
    if [ -z "$p95_ms" ] || ! awk -v actual="$p95_ms" -v budget="$budget_ms" \
        'BEGIN { exit !(actual <= budget) }'
    then
        echo "$range_name p95 ${p95_ms:-missing} ms exceeds ${budget_ms} ms" >&2
        exit 1
    fi
    echo "$range_name samples=$samples p50_ms=$p50_ms p95_ms=$p95_ms budget_ms=$budget_ms"
}

check_projection_after_writer() {
    writer_name=$1
    projection_name=$2
    counts=$(query_one "
        select
          (select count(*) from slice where name = '$writer_name'),
          (select count(*) from slice where name = '$projection_name');
    ")
    old_ifs=$IFS
    IFS=,
    set -- $counts
    IFS=$old_ifs
    writer_samples=${1:-0}
    projection_samples=${2:-0}
    if [ "$writer_samples" -lt "$minimum_samples" ] ||
        [ "$writer_samples" -ne "$projection_samples" ]
    then
        echo "$writer_name samples=$writer_samples but $projection_name samples=$projection_samples" >&2
        exit 1
    fi
    violations=$(query_one "
        with writers as (
          select
            row_number() over (order by ts) as sample,
            ts + dur as completed_at
          from slice
          where name = '$writer_name'
        ),
        projections as (
          select
            row_number() over (order by ts) as sample,
            ts as started_at
          from slice
          where name = '$projection_name'
        )
        select count(*)
        from writers
        join projections using (sample)
        where projections.started_at < writers.completed_at;
    ")
    if [ "$violations" -ne 0 ]; then
        echo "$projection_name overlaps $writer_name in $violations samples" >&2
        exit 1
    fi
    echo "$projection_name starts after $writer_name in every sample"
}

check_range \
    "ait.server.history_promotion.perf.prepare_10" \
    "$history_prepare_budget_ms"
check_range \
    "ait.server.history_promotion.writer_critical_section" \
    "$history_writer_budget_ms"
check_range \
    "ait.server.task_land.perf.receipts_plus_aggregate_10" \
    "$task_land_budget_ms"
check_range \
    "ait.server.task_land.atomic.writer_critical_section" \
    "$task_land_writer_budget_ms"

check_projection_after_writer \
    "ait.server.history_promotion.writer_critical_section" \
    "ait.server.history_promotion.response_projection"
check_projection_after_writer \
    "ait.server.task_land.atomic.writer_critical_section" \
    "ait.server.task_land.atomic.response_projection"

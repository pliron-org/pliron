#!/usr/bin/env bash
set -euo pipefail

# Wrapper for use as CC=... in existing build systems.
# It transparently runs:
#   clang (to LLVM IR) -> llvm-opt (--opts o1) -> clang (final output)
# for C-source invocations, and falls back to plain clang otherwise.
#
# Usage:
#   make CC=/path/to/llvm-opt-wrapper.sh \
#        LLVM_OPT=/path/to/llvm-opt
#
# Optional overrides:
#   CLANG=/path/to/clang
#   LLVM_OPT=/path/to/llvm-opt
#
# Direct invocation (clang-compatible arguments):
#   ./llvm-opt-wrapper.sh -O2 -g -c foo.c -o foo.o
#
# Notes:
# - Interception routes C compilation (both compile-only `-c` and compile-and-link)
#   through llvm-opt on a per-file basis.
# - Link steps without C sources and other non-matching invocations are forwarded unchanged.

CLANG_BIN="${CLANG:-clang}"
LLVM_OPT_BIN="${LLVM_OPT:-llvm-opt}"

if ! command -v "$CLANG_BIN" >/dev/null 2>&1; then
    cat >&2 <<EOF
llvm-opt-wrapper: clang binary not found: '$CLANG_BIN'

Set CLANG to the compiler to use, for example:
  CLANG=/path/to/clang ./llvm-opt-wrapper.sh <clang-args>
  make CC=/path/to/llvm-opt-wrapper.sh CLANG=/path/to/clang LLVM_OPT=/path/to/llvm-opt
EOF
    exit 1
fi

if ! command -v "$LLVM_OPT_BIN" >/dev/null 2>&1; then
    cat >&2 <<EOF
llvm-opt-wrapper: llvm-opt binary not found: '$LLVM_OPT_BIN'

Set LLVM_OPT to the llvm-opt binary, for example:
  LLVM_OPT=/path/to/llvm-opt ./llvm-opt-wrapper.sh <clang-args>
  make CC=/path/to/llvm-opt-wrapper.sh LLVM_OPT=/path/to/llvm-opt
EOF
    exit 1
fi

if [[ $# -eq 0 ]]; then
    exec "$CLANG_BIN"
fi

orig_args=("$@")

source_files=()
has_compile_only=0
has_explicit_output=0
explicit_output_file=""

# Track source files and basic mode flags.
i=0
while [[ $i -lt ${#orig_args[@]} ]]; do
    arg="${orig_args[$i]}"
    case "$arg" in
        -c)
            has_compile_only=1
            ;;
        -o)
            has_explicit_output=1
            if [[ $((i + 1)) -lt ${#orig_args[@]} ]]; then
                explicit_output_file="${orig_args[$((i + 1))]}"
            fi
            ;;
        -o*)
            has_explicit_output=1
            explicit_output_file="${arg#-o}"
            ;;
        *.c)
            source_files+=("$arg")
            ;;
    esac
    i=$((i + 1))
done

# If there are no C sources, behave exactly as clang (e.g., pure link step).
if [[ ${#source_files[@]} -eq 0 ]]; then
    exec "$CLANG_BIN" "${orig_args[@]}"
fi

# In compile-only mode (-c), clang disallows multiple sources with an explicit -o.
if [[ "$has_compile_only" -eq 1 && "$has_explicit_output" -eq 1 && ${#source_files[@]} -gt 1 ]]; then
    exec "$CLANG_BIN" "${orig_args[@]}"
fi

workdir="$(mktemp -d "${TMPDIR:-/tmp}/llvm-opt-wrapper.XXXXXX")"
trap 'rm -rf "$workdir"' EXIT

# Build base emit args for IR emission by removing original -o <file>, -c, and all source files.
emit_args=()
i=0
while [[ $i -lt ${#orig_args[@]} ]]; do
    arg="${orig_args[$i]}"
    if [[ "$arg" == "-c" ]]; then
        i=$((i + 1))
        continue
    fi
    if [[ "$arg" == "-o" ]]; then
        i=$((i + 2))
        continue
    fi
    if [[ "$arg" == -o* && "$arg" != "-o" ]]; then
        i=$((i + 1))
        continue
    fi
    is_src=0
    for src in "${source_files[@]}"; do
        if [[ "$arg" == "$src" ]]; then
            is_src=1
            break
        fi
    done
    if [[ "$is_src" -eq 1 ]]; then
        i=$((i + 1))
        continue
    fi
    emit_args+=("$arg")
    i=$((i + 1))
done

generated_objs=()

for idx in "${!source_files[@]}"; do
    src="${source_files[$idx]}"
    base="$(basename "${src%.c}")"
    input_ll="$workdir/${base}_${idx}.ll"
    opt_ll="$workdir/${base}_${idx}.opt.ll"

    if [[ "$has_compile_only" -eq 1 ]]; then
        if [[ "$has_explicit_output" -eq 1 ]]; then
            obj="$explicit_output_file"
        else
            obj="${src%.c}.o"
        fi
    else
        obj="$workdir/${base}_${idx}.o"
    fi
    generated_objs+=("$obj")

    # Step 1: C -> LLVM IR
    "$CLANG_BIN" "${emit_args[@]}" "$src" -S -emit-llvm -O0 -o "$input_ll"
    
    # Step 2: Optimize with llvm-opt
    "$LLVM_OPT_BIN" -S -i "$input_ll" -o "$opt_ll" --opts o1
    
    # Step 3: LLVM IR -> Object file (.o)
    "$CLANG_BIN" "${emit_args[@]}" -c "$opt_ll" -o "$obj"
done

# If this was a compile-only command (-c), our work is done.
if [[ "$has_compile_only" -eq 1 ]]; then
    exit 0
fi

# Step 4: For compile-and-link commands, link all generated .o files along with other arguments.
link_args=()
i=0
while [[ $i -lt ${#orig_args[@]} ]]; do
    arg="${orig_args[$i]}"
    is_src=0
    for src in "${source_files[@]}"; do
        if [[ "$arg" == "$src" ]]; then
            is_src=1
            break
        fi
    done
    if [[ "$is_src" -eq 1 ]]; then
        i=$((i + 1))
        continue
    fi
    link_args+=("$arg")
    i=$((i + 1))
done

exec "$CLANG_BIN" "${link_args[@]}" "${generated_objs[@]}"

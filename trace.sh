#!/bin/bash -ex
ARGS=$#
if [[ "$ARGS" -ne 1 ]]; then
    echo "usage: trace.sh path/to/program.asm"
    echo "generates program.bin and program.log in the same directory."
    echo "program.bin is generated via the assembler, and program.log is generated via the remote chiplab."
    exit 1
fi

ASM=$1
OUT=$(echo $ASM | sed 's/\.asm/\.bin/')
LOG=$(echo $ASM | sed 's/\.asm/\.log/')
echo "assembling $ASM to $OUT"

cargo run --manifest-path 6502_asm/Cargo.toml $ASM $OUT

echo "running on chiplab to $LOG"
curl -F "file=@${OUT}" https://chiplab.emulationonline.com/6502/blob --output $LOG

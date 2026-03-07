db 0

org $300
.reset:
  ldx #$FA
  txs  ; initialize sp
  ldx #$FF
.mainloop:
  phx
  jmp .mainloop

.loop:
  jmp .loop

.nmi:
  rti

.irq:
  rti

; interrupt vectors
org $FFFA
dw .nmi
dw .reset
dw .irq


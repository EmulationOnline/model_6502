db 0

org $300
.reset:
  ldx #$fa
.mainloop:
  stx $cafe
  inx
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


db 0

org $300
.reset:
  ldx #$10
.countdown:
  stx $cafe
  stx $dead
  jmp .countdown

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


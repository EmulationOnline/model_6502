db 0

org $300
.reset:
  ldx #$05
.mainloop:
  stx $cafe
  dex
  php
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


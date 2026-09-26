; A small program for the .dbg fixture: a reset routine, a loop that
; clears RAM, a call into another module and a table it reads.
.p816
.include "macros.inc"

.import ClearPalette, Palette

.zeropage
frame:  .res 2

.bss
buffer: .res 16

.code
.proc Reset
        sei
        clc
        xce
        a8
        .i16
        rep #$10
        ldx #$1FFF
        txs
        brightness $80          ; forced blank while we set up
        jsr ClearBuffer
        jsr ClearPalette
        brightness $0F
@wait:  wai
        inc frame
        bra @wait
.endproc

; Zero the 16 bytes of buffer.
.proc ClearBuffer
        ldx #15
@loop:  stz buffer,x
        dex
        bpl @loop
        rts
.endproc

.proc Nmi
        rti
.endproc

.segment "HEADER"
        .byte "DBG FIXTURE          "   ; title, 21 bytes
        .byte $20                       ; LoROM
        .byte $00                       ; ROM only
        .byte $05                       ; 32 KB
        .byte $00                       ; no RAM
        .byte $01                       ; North America
        .byte $00                       ; developer
        .byte $00                       ; version
        .word $FFFF, $0000              ; checksum complement, checksum
        ; native vectors
        .word 0, 0, Nmi, Nmi, 0, Nmi, 0, Nmi
        ; emulation vectors
        .word 0, 0, Nmi, 0, 0, 0, Reset, Nmi

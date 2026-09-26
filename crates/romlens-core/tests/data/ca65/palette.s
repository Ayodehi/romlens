; The second module: clears CGRAM from a table.
.p816
.include "macros.inc"

.export ClearPalette, Palette

.code
; Called from Reset with an 8-bit accumulator and 16-bit index registers,
; which ca65 must be told here: each module starts from its defaults.
.a8
.i16
.proc ClearPalette
        stz $2121
        ldx #0
@next:  lda Palette,x
        sta $2122
        inx
        cpx #Palette_size
        bne @next
        rts
.endproc

.rodata
Palette:
        .word $0000, $7FFF, $001F, $03E0
Palette_size = * - Palette

; Macros the program uses.

; Set the screen's brightness, 0 to 15, with the screen on.
.macro brightness level
        lda #level
        sta $2100
.endmacro

; A 16-bit accumulator.
.macro a16
        rep #$20
        .a16
.endmacro

; An 8-bit accumulator.
.macro a8
        sep #$20
        .a8
.endmacro

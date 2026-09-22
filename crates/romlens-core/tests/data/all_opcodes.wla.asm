; Every 65816 opcode once, for the WLA-DX round trip (tests/wla_roundtrip.rs).
; Generated from tests/data/opcodes_65816.txt; operands are 12 34 56 truncated
; to the width under 16-bit A and X/Y. Branch operands are the displacements
; themselves ($12 / $3412), which is how WLA-DX reads a numeric branch operand.
.MEMORYMAP
SLOTSIZE $8000
DEFAULTSLOT 0
SLOT 0 $8000
.ENDME
.ROMBANKMAP
BANKSTOTAL 1
BANKSIZE $8000
BANKS 1
.ENDRO
.BANK 0 SLOT 0
.ORG 0
.ACCU 16
.INDEX 16

BRK $12                  ; $00
ORA.B ($12,X)           ; $01
COP $12                  ; $02
ORA.B $12,S             ; $03
TSB.B $12               ; $04
ORA.B $12               ; $05
ASL.B $12               ; $06
ORA.B [$12]             ; $07
PHP                     ; $08
ORA #$3412              ; $09
ASL A                     ; $0A
PHD                     ; $0B
TSB.W $3412             ; $0C
ORA.W $3412             ; $0D
ASL.W $3412             ; $0E
ORA.L $563412           ; $0F
BPL $12                 ; $10
ORA.B ($12),Y           ; $11
ORA.B ($12)             ; $12
ORA.B ($12,S),Y         ; $13
TRB.B $12               ; $14
ORA.B $12,X             ; $15
ASL.B $12,X             ; $16
ORA.B [$12],Y           ; $17
CLC                     ; $18
ORA.W $3412,Y           ; $19
INC A                     ; $1A
TCS                     ; $1B
TRB.W $3412             ; $1C
ORA.W $3412,X           ; $1D
ASL.W $3412,X           ; $1E
ORA.L $563412,X         ; $1F
JSR.W $3412             ; $20
AND.B ($12,X)           ; $21
JSL.L $563412           ; $22
AND.B $12,S             ; $23
BIT.B $12               ; $24
AND.B $12               ; $25
ROL.B $12               ; $26
AND.B [$12]             ; $27
PLP                     ; $28
AND #$3412              ; $29
ROL A                     ; $2A
PLD                     ; $2B
BIT.W $3412             ; $2C
AND.W $3412             ; $2D
ROL.W $3412             ; $2E
AND.L $563412           ; $2F
BMI $12                 ; $30
AND.B ($12),Y           ; $31
AND.B ($12)             ; $32
AND.B ($12,S),Y         ; $33
BIT.B $12,X             ; $34
AND.B $12,X             ; $35
ROL.B $12,X             ; $36
AND.B [$12],Y           ; $37
SEC                     ; $38
AND.W $3412,Y           ; $39
DEC A                     ; $3A
TSC                     ; $3B
BIT.W $3412,X           ; $3C
AND.W $3412,X           ; $3D
ROL.W $3412,X           ; $3E
AND.L $563412,X         ; $3F
RTI                     ; $40
EOR.B ($12,X)           ; $41
WDM $12                  ; $42
EOR.B $12,S             ; $43
MVP $34,$12             ; $44
EOR.B $12               ; $45
LSR.B $12               ; $46
EOR.B [$12]             ; $47
PHA                     ; $48
EOR #$3412              ; $49
LSR A                     ; $4A
PHK                     ; $4B
JMP.W $3412             ; $4C
EOR.W $3412             ; $4D
LSR.W $3412             ; $4E
EOR.L $563412           ; $4F
BVC $12                 ; $50
EOR.B ($12),Y           ; $51
EOR.B ($12)             ; $52
EOR.B ($12,S),Y         ; $53
MVN $34,$12             ; $54
EOR.B $12,X             ; $55
LSR.B $12,X             ; $56
EOR.B [$12],Y           ; $57
CLI                     ; $58
EOR.W $3412,Y           ; $59
PHY                     ; $5A
TCD                     ; $5B
JML.L $563412           ; $5C
EOR.W $3412,X           ; $5D
LSR.W $3412,X           ; $5E
EOR.L $563412,X         ; $5F
RTS                     ; $60
ADC.B ($12,X)           ; $61
PER $3412               ; $62
ADC.B $12,S             ; $63
STZ.B $12               ; $64
ADC.B $12               ; $65
ROR.B $12               ; $66
ADC.B [$12]             ; $67
PLA                     ; $68
ADC #$3412              ; $69
ROR A                     ; $6A
RTL                     ; $6B
JMP.W ($3412)           ; $6C
ADC.W $3412             ; $6D
ROR.W $3412             ; $6E
ADC.L $563412           ; $6F
BVS $12                 ; $70
ADC.B ($12),Y           ; $71
ADC.B ($12)             ; $72
ADC.B ($12,S),Y         ; $73
STZ.B $12,X             ; $74
ADC.B $12,X             ; $75
ROR.B $12,X             ; $76
ADC.B [$12],Y           ; $77
SEI                     ; $78
ADC.W $3412,Y           ; $79
PLY                     ; $7A
TDC                     ; $7B
JMP.W ($3412,X)         ; $7C
ADC.W $3412,X           ; $7D
ROR.W $3412,X           ; $7E
ADC.L $563412,X         ; $7F
BRA $12                 ; $80
STA.B ($12,X)           ; $81
BRL $3412               ; $82
STA.B $12,S             ; $83
STY.B $12               ; $84
STA.B $12               ; $85
STX.B $12               ; $86
STA.B [$12]             ; $87
DEY                     ; $88
BIT #$3412              ; $89
TXA                     ; $8A
PHB                     ; $8B
STY.W $3412             ; $8C
STA.W $3412             ; $8D
STX.W $3412             ; $8E
STA.L $563412           ; $8F
BCC $12                 ; $90
STA.B ($12),Y           ; $91
STA.B ($12)             ; $92
STA.B ($12,S),Y         ; $93
STY.B $12,X             ; $94
STA.B $12,X             ; $95
STX.B $12,Y             ; $96
STA.B [$12],Y           ; $97
TYA                     ; $98
STA.W $3412,Y           ; $99
TXS                     ; $9A
TXY                     ; $9B
STZ.W $3412             ; $9C
STA.W $3412,X           ; $9D
STZ.W $3412,X           ; $9E
STA.L $563412,X         ; $9F
LDY #$3412              ; $A0
LDA.B ($12,X)           ; $A1
LDX #$3412              ; $A2
LDA.B $12,S             ; $A3
LDY.B $12               ; $A4
LDA.B $12               ; $A5
LDX.B $12               ; $A6
LDA.B [$12]             ; $A7
TAY                     ; $A8
LDA #$3412              ; $A9
TAX                     ; $AA
PLB                     ; $AB
LDY.W $3412             ; $AC
LDA.W $3412             ; $AD
LDX.W $3412             ; $AE
LDA.L $563412           ; $AF
BCS $12                 ; $B0
LDA.B ($12),Y           ; $B1
LDA.B ($12)             ; $B2
LDA.B ($12,S),Y         ; $B3
LDY.B $12,X             ; $B4
LDA.B $12,X             ; $B5
LDX.B $12,Y             ; $B6
LDA.B [$12],Y           ; $B7
CLV                     ; $B8
LDA.W $3412,Y           ; $B9
TSX                     ; $BA
TYX                     ; $BB
LDY.W $3412,X           ; $BC
LDA.W $3412,X           ; $BD
LDX.W $3412,Y           ; $BE
LDA.L $563412,X         ; $BF
CPY #$3412              ; $C0
CMP.B ($12,X)           ; $C1
REP #$12                ; $C2
CMP.B $12,S             ; $C3
CPY.B $12               ; $C4
CMP.B $12               ; $C5
DEC.B $12               ; $C6
CMP.B [$12]             ; $C7
INY                     ; $C8
CMP #$3412              ; $C9
DEX                     ; $CA
WAI                     ; $CB
CPY.W $3412             ; $CC
CMP.W $3412             ; $CD
DEC.W $3412             ; $CE
CMP.L $563412           ; $CF
BNE $12                 ; $D0
CMP.B ($12),Y           ; $D1
CMP.B ($12)             ; $D2
CMP.B ($12,S),Y         ; $D3
PEI.B ($12)             ; $D4
CMP.B $12,X             ; $D5
DEC.B $12,X             ; $D6
CMP.B [$12],Y           ; $D7
CLD                     ; $D8
CMP.W $3412,Y           ; $D9
PHX                     ; $DA
STP                     ; $DB
JML.W [$3412]           ; $DC
CMP.W $3412,X           ; $DD
DEC.W $3412,X           ; $DE
CMP.L $563412,X         ; $DF
CPX #$3412              ; $E0
SBC.B ($12,X)           ; $E1
SEP #$12                ; $E2
SBC.B $12,S             ; $E3
CPX.B $12               ; $E4
SBC.B $12               ; $E5
INC.B $12               ; $E6
SBC.B [$12]             ; $E7
INX                     ; $E8
SBC #$3412              ; $E9
NOP                     ; $EA
XBA                     ; $EB
CPX.W $3412             ; $EC
SBC.W $3412             ; $ED
INC.W $3412             ; $EE
SBC.L $563412           ; $EF
BEQ $12                 ; $F0
SBC.B ($12),Y           ; $F1
SBC.B ($12)             ; $F2
SBC.B ($12,S),Y         ; $F3
PEA.W $3412             ; $F4
SBC.B $12,X             ; $F5
INC.B $12,X             ; $F6
SBC.B [$12],Y           ; $F7
SED                     ; $F8
SBC.W $3412,Y           ; $F9
PLX                     ; $FA
XCE                     ; $FB
JSR.W ($3412,X)         ; $FC
SBC.W $3412,X           ; $FD
INC.W $3412,X           ; $FE
SBC.L $563412,X         ; $FF

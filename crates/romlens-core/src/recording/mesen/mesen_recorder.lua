-- Romlens recorder for Mesen (2.x and MesenCE).
--
-- Writes a raw stream of the machine at every frame end. `romlens rec pack`
-- turns it into a .romrec recording; this script only captures and leaves
-- compression, hashing and the register layouts to Romlens.
--
-- Needs Mesen's "Allow access to I/O and OS functions" script option.
-- Output: $ROMLENS_REC_OUT, or romlens-<time>.rlstream in the script data
-- folder. $ROMLENS_REC_FRAMES stops the emulator after that many frames.
--
-- Stream format 1, little-endian (docs/13, "The Mesen stream"):
--   header  "RLSTREAM", u16 version, s2 producer, s2 ROM SHA-1,
--           i8 created (Unix seconds), u32 ROM size, 64 x 16 bytes of ROM
--           at offsets size / 64 * i, u16 field count, s1 each field name
--   'F'     u32 frame, u16 changed fields then (u16 index, i8 value) each,
--           52 bytes $2100-$2133 last written, 52 bytes seen flags,
--           128 bytes $4300-$437F, then per memory region
--           (vram, cgram, oam, wram) u16 changed blocks then
--           (u16 block, the block's bytes) each
--   'D'     u32 frame, u8 value written to $420B, u16 scanline,
--           128 bytes $4300-$437F at the moment of the write
--   'L'     u32 frame: a savestate was loaded before this frame
--   'E'     u32 frames written, the stream's clean end

local M = emu.memType
local VERSION = 1
local BLOCK = 256
local REGISTER_FORMAT = "<" .. string.rep("B", 104)

local out_path = os.getenv("ROMLENS_REC_OUT")
if not out_path or out_path == "" then
  out_path = emu.getScriptDataFolder() .. "/romlens-" .. os.date("%Y%m%d-%H%M%S") .. ".rlstream"
end
emu.log(out_path)
local frame_limit = tonumber(os.getenv("ROMLENS_REC_FRAMES") or "")

local out = assert(io.open(out_path, "wb"))
out:setvbuf("full", 1 << 20)

-- Memory regions in stream order: memory type, size.
local regions = {
  { M.snesVideoRam, 0x10000 },
  { M.snesCgRam, 0x200 },
  { M.snesSpriteRam, 0x220 },
  { M.snesWorkRam, 0x20000 },
}

local formats = {}
local function block_format(len)
  formats[len] = formats[len] or ("<" .. string.rep("I4", len // 4))
  return formats[len]
end

local words = {}
local function read_block(mem, base, len)
  for i = 1, len // 4 do
    words[i] = emu.read32(base + (i - 1) * 4, mem)
  end
  return string.pack(block_format(len), table.unpack(words, 1, len // 4))
end

-- The DMA registers, read without side effects.
local function dma_registers()
  local parts = {}
  for i = 0, 31 do
    parts[i + 1] = string.pack("<I4", emu.read32(0x4300 + i * 4, M.snesDebug))
  end
  return table.concat(parts)
end

-- The getState() fields worth keeping: numbers and booleans under these
-- prefixes. Romlens picks the ones it knows by name, so a Mesen version that
-- exports more needs no change here.
local prefixes = { "cpu.", "ppu.", "internalRegisters.", "dmaController.", "memoryManager.hClock", "frameCount", "masterClock" }
local function wanted(key)
  for _, p in ipairs(prefixes) do
    if key:sub(1, #p) == p then return true end
  end
  return false
end

local function value_of(v)
  if v == true then return 1 elseif v == false then return 0 end
  return math.tointeger(v) or 0
end

local fields, previous_values = {}, {}
do
  for k, v in pairs(emu.getState()) do
    if wanted(k) and (type(v) == "number" or type(v) == "boolean") then
      fields[#fields + 1] = k
    end
  end
  table.sort(fields)
end

-- Which ROM: Mesen offers only SHA-1, so also its size and 64 samples of 16
-- bytes spread through it, which Romlens checks against the ROM it is given.
local rom = emu.getRomInfo()
local rom_size = emu.getMemorySize(M.snesPrgRom)
local samples = {}
for i = 0, 63 do
  local at = (rom_size // 64) * i
  for w = 0, 3 do
    samples[#samples + 1] = string.pack("<I4", emu.read32(at + w * 4, M.snesPrgRom))
  end
end
local header = { "RLSTREAM", string.pack("<I2", VERSION), string.pack("<s2", "Mesen"),
  string.pack("<s2", rom.fileSha1Hash or ""), string.pack("<i8", os.time()),
  string.pack("<I4", rom_size), table.concat(samples),
  string.pack("<I2", #fields) }
for _, k in ipairs(fields) do header[#header + 1] = string.pack("<s1", k) end
out:write(table.concat(header))

-- $2100-$2133, the last byte the game wrote to each. A callback on the
-- register memory type sees a write through any bank mirror ($00-$3F,
-- $80-$BF), including DMA's B-bus writes, so two callbacks cover what would
-- otherwise take one per bank.
local last, seen = {}, {}
local function reset_registers()
  for i = 0, 0x33 do last[i], seen[i] = 0, 0 end
end
reset_registers()
local function on_ppu_write(address, value)
  local r = address & 0xFF
  last[r], seen[r] = value, 1
end

local frame = 0
local finished = false
local finish

-- A script error must not leave the emulator hung or the stream half-written.
local function guarded(fn)
  return function(...)
    local ok, err = pcall(fn, ...)
    if not ok then
      emu.log("Romlens recorder failed: " .. tostring(err))
      pcall(finish)
      emu.stop(3)
    end
  end
end

local function on_dma(_, value)
  local s = emu.getState()
  out:write("D", string.pack("<I4I1I2", frame, value, value_of(s["ppu.scanline"])), dma_registers())
end

local cpu = emu.cpuType.snes
emu.addMemoryCallback(guarded(on_ppu_write), emu.callbackType.write, 0x2100, 0x2133, cpu, M.snesRegister)
emu.addMemoryCallback(guarded(on_dma), emu.callbackType.write, 0x420B, 0x420B, cpu, M.snesRegister)

local previous = {}

local function write_frame()
  local state = emu.getState()
  local changed = {}
  for i, k in ipairs(fields) do
    local v = value_of(state[k])
    if previous_values[i] ~= v then
      previous_values[i] = v
      changed[#changed + 1] = string.pack("<I2i8", i - 1, v)
    end
  end
  local parts = { "F", string.pack("<I4I2", frame, #changed), table.concat(changed) }
  local regs = {}
  for i = 0, 0x33 do
    regs[i + 1] = last[i]
    regs[i + 53] = seen[i]
  end
  parts[#parts + 1] = string.pack(REGISTER_FORMAT, table.unpack(regs, 1, 104))
  parts[#parts + 1] = dma_registers()
  for r, region in ipairs(regions) do
    local mem, size = region[1], region[2]
    previous[r] = previous[r] or {}
    local blocks, count = {}, 0
    for b = 0, (size - 1) // BLOCK do
      local len = math.min(BLOCK, size - b * BLOCK)
      local bytes = read_block(mem, b * BLOCK, len)
      if previous[r][b] ~= bytes then
        previous[r][b] = bytes
        count = count + 1
        blocks[count] = string.pack("<I2", b) .. bytes
      end
    end
    parts[#parts + 1] = string.pack("<I2", count)
    parts[#parts + 1] = table.concat(blocks)
  end
  out:write(table.concat(parts))
  frame = frame + 1
  if frame % 60 == 0 then out:flush() end
end

function finish()
  if finished then return end
  finished = true
  out:write("E", string.pack("<I4", frame))
  out:close()
  emu.log("Romlens recorder: " .. frame .. " frames to " .. out_path)
end

emu.addEventCallback(guarded(function()
  if finished then return end
  write_frame()
  if frame_limit and frame >= frame_limit then
    finish()
    emu.stop(0)
  end
end), emu.eventType.endFrame)

emu.addEventCallback(guarded(function()
  -- The loaded state's registers were written before we could see them.
  reset_registers()
  out:write("L", string.pack("<I4", frame))
end), emu.eventType.stateLoaded)

emu.addEventCallback(guarded(finish), emu.eventType.scriptEnded)

emu.log("Romlens recorder: writing " .. out_path)

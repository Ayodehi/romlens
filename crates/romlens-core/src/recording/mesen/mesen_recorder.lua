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
-- Where Mesen has an execution log (emu.startExecutionLog, in the MesenCE
-- fork), the script also records one and writes it beside the stream as
-- <name>.mxlog: every minute, so a crash loses little, and at the end. Import
-- it into Romlens as a trace (docs/17). Elsewhere the script records the
-- stream alone, as before.
--
-- Live: with Mesen's network access on as well, the script also sends the
-- stream to Romlens over a local connection (127.0.0.1, port 7462 or
-- $ROMLENS_LIVE_PORT) whenever Romlens is listening (File › Start Live
-- Session), trying every two seconds. A new connection gets the header and a
-- full frame first, then the same deltas the file gets. Sends never block the
-- game; a connection that falls 16 MB behind is dropped, and the file
-- recording carries on regardless. With the fork's takeExecutionLogDelta,
-- the connection also gets the execution log: all of it on connecting, then
-- once a second what the CPU did since ('X' records), so Romlens can fill in
-- the disassembly as the game plays.
--
-- The sound side (stream version 3, docs/23): where this Mesen can read the
-- sound CPU's RAM and the DSP's registers (every 2.x can), the script also
-- records every byte the game writes to the four APU ports, every write the
-- SPC700 makes to its I/O registers (the DSP's among them), each on the
-- SPC700's own clock, and at each frame end audio RAM's changed blocks and
-- the DSP's registers.
--
-- Stream format 3, little-endian (docs/13, "The Mesen stream"):
--   header  "RLSTREAM", u16 version, s2 producer, s2 ROM SHA-1,
--           i8 created (Unix seconds), u32 ROM size, 64 x 16 bytes of ROM
--           at offsets size / 64 * i, u16 field count, s1 each field name,
--           u8 flags (1 = the sound side)
--   'F'     u32 frame, u16 changed fields then (u16 index, i8 value) each,
--           52 bytes $2100-$2133 last written, 52 bytes seen flags,
--           128 bytes $4300-$437F, then per memory region
--           (vram, cgram, oam, wram) u16 changed blocks then
--           (u16 block, the block's bytes) each
--   'D'     u32 frame, u8 value written to $420B, u16 scanline,
--           128 bytes $4300-$437F at the moment of the write, then
--           u16 master cycles into the line, u16 VMADD, u8 CGADD,
--           u16 OAM address, u32 WRAM port address, u8 K, u16 PC
--   'R'     u32 frame, u32 count, then (i16 scanline, u16 dot, u8 register
--           - $2100, u8 value) for each PPU register write made while the
--           frame was drawn; the scanline counts from the frame's first
--           line, the vertical blank before it negative. Written before
--           the frame's 'F', from the second frame on
--   'A'     u32 frame, u32 count, then (u8 kind, u8 address, u8 value,
--           u64 SPC700 cycle, u64 master clock) each: kind 0 the game wrote
--           $2140 + address, kind 1 the SPC700 wrote $F0 + address. Before
--           the frame's 'F', with the sound side only
--   'S'     u32 frame, u64 SPC700 cycle, u16 changed audio RAM blocks then
--           (u16 block, 256 bytes) each, 128 bytes of DSP registers. After
--           the frame's 'A'
--   'L'     u32 frame: a savestate was loaded before this frame
--   'E'     u32 frames written, the stream's clean end

local M = emu.memType
local VERSION = 3
local BLOCK = 256
local REGISTER_FORMAT = "<" .. string.rep("B", 104)

-- Mesen gives a script `io` and `os` only with the script window's
-- "Allow access to I/O and OS functions" setting on; without them there is
-- nowhere to write the stream, so say so and do nothing else.
if not io or not os then
  local why = "the Romlens recorder needs file access: in the script window, open Settings, "
    .. "turn on \"Allow access to I/O and OS functions\", and run the script again"
  emu.log(why)
  pcall(emu.displayMessage, "Romlens", why)
  return
end

local out_path = os.getenv("ROMLENS_REC_OUT")
if not out_path or out_path == "" then
  out_path = emu.getScriptDataFolder() .. "/romlens-" .. os.date("%Y%m%d-%H%M%S") .. ".rlstream"
end
emu.log(out_path)
local frame_limit = tonumber(os.getenv("ROMLENS_REC_FRAMES") or "")

local out = assert(io.open(out_path, "wb"))
out:setvbuf("full", 1 << 20)

-- The execution log, when this Mesen has one.
local xlog_path = out_path:gsub("%.rlstream$", "") .. ".mxlog"
local xlog = emu.startExecutionLog ~= nil and emu.getExecutionLog ~= nil
if xlog then
  emu.startExecutionLog()
end

-- Written to a .part file and renamed, so a reader never sees half a log.
local function write_xlog()
  if not xlog then return end
  local data = emu.getExecutionLog()
  local part = xlog_path .. ".part"
  local f = assert(io.open(part, "wb"))
  f:write(data)
  f:close()
  os.remove(xlog_path)
  assert(os.rename(part, xlog_path))
end

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
local prefixes = { "cpu.", "ppu.", "internalRegisters.", "dmaController.", "memoryManager.hClock", "frameCount", "masterClock", "spc." }
local function wanted(key)
  -- The DSP's internal state is large and changes every sample; its
  -- registers come whole in the 'S' record instead.
  if key:sub(1, 8) == "spc.dsp." then return false end
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

-- The sound side, where this Mesen has it.
local audio = M.spcRam ~= nil and M.spcDspRegisters ~= nil and emu.cpuType.spc ~= nil
  and emu.getCpuCycleCount ~= nil
header[#header + 1] = string.pack("<B", audio and 1 or 0)
local header_bytes = table.concat(header)
out:write(header_bytes)

-- The live connection, when there is one: { sock, pending, fresh }.
local LIVE_PORT = tonumber(os.getenv("ROMLENS_LIVE_PORT") or "") or 7462
local LIVE_BACKLOG = 16 << 20
local has_socket, socket = pcall(require, "socket.core")
local live = nil

local function live_close(why)
  if not live then return end
  pcall(live.sock.close, live.sock)
  live = nil
  if why then emu.log("Romlens recorder: live connection closed: " .. why) end
end

local function live_flush()
  while live and #live.pending > 0 do
    local last, err, partial = live.sock:send(live.pending)
    local sent = last or partial or 0
    if sent > 0 then live.pending = live.pending:sub(sent + 1) end
    if err == "timeout" then break end
    if err then return live_close(err) end
  end
  if live and #live.pending > LIVE_BACKLOG then live_close("Romlens is not keeping up") end
end

local function live_send(data)
  if not live then return end
  live.pending = live.pending .. data
  live_flush()
end

local function live_try_connect()
  if not has_socket or live then return end
  local c = socket.tcp()
  if not c then return end
  c:settimeout(0.02)
  if not c:connect("127.0.0.1", LIVE_PORT) then
    c:close()
    return
  end
  c:settimeout(0)
  pcall(c.setoption, c, "tcp-nodelay", true)
  live = { sock = c, pending = header_bytes, fresh = true }
  emu.log("Romlens recorder: streaming live to Romlens on port " .. LIVE_PORT)
end

-- The execution log over the live connection: 'X', u32 length, the log.
local xlog_live = xlog and emu.takeExecutionLogDelta ~= nil
local function live_xlog(data)
  live_send("X" .. string.pack("<I4", #data) .. data)
end

-- $2100-$2133, the last byte the game wrote to each. A callback on the
-- register memory type sees a write through any bank mirror ($00-$3F,
-- $80-$BF), including DMA's B-bus writes, so two callbacks cover what would
-- otherwise take one per bank.
local last, seen = {}, {}
local function reset_registers()
  for i = 0, 0x33 do last[i], seen[i] = 0, 0 end
end
reset_registers()
local frame = 0

-- Each write's master clock, placed on its scanline at the frame's end:
-- the clock and scanline of the previous frame end give the line and dot.
-- Both clocks come from getMasterClock: getState's masterClock is 32 bits
-- and wraps to 0 after about 13,700 frames, which would put every later
-- write billions of cycles away.
local ends = nil
local w_clock, w_reg, w_value, w_count = {}, {}, {}, 0
local get_clock = emu.getMasterClock
local function on_ppu_write(address, value)
  local r = address & 0xFF
  last[r], seen[r] = value, 1
  if ends then
    w_count = w_count + 1
    w_clock[w_count], w_reg[w_count], w_value[w_count] = get_clock(), r, value
  end
end

local function line_writes(state)
  local now = get_clock()
  local line = value_of(state["ppu.scanline"])
  local h = value_of(state["memoryManager.hClock"])
  local r = nil
  -- The lines between the two ends: a frame's length. Anything else (a
  -- power cycle restarting the clock, a rewind, a paused and stepped
  -- frame) leaves the frame without line writes rather than guessing.
  local total = ends and math.tointeger((now - ends.clock + 682) // 1364)
  if total and total >= 200 and total <= 400 then
    local parts, count = {}, 0
    for i = 1, w_count do
      local pos = math.tointeger(ends.line * 1364 + ends.h + (w_clock[i] - ends.clock))
      local l = pos and pos // 1364 - total
      if l and l >= -400 and l <= 400 then
        count = count + 1
        parts[count] = string.pack("<i2I2BB", l, (pos % 1364) // 4, w_reg[i], w_value[i])
      end
    end
    r = "R" .. string.pack("<I4I4", frame, count) .. table.concat(parts)
  end
  ends = { clock = now, line = line, h = h }
  w_count = 0
  return r
end

local finished = false
local finish

-- A script error must not leave the emulator hung or the stream half-written.
-- A failure stops the recording, never the game: it is shown and logged
-- with where it happened, and the stream is closed cleanly. Only a
-- headless run with a frame limit stops the emulator, so it cannot hang.
local failed = false
local function guarded(fn)
  return function(...)
    if failed then return end
    local ok, err = xpcall(fn, debug.traceback, ...)
    if not ok then
      failed = true
      emu.log("Romlens recorder failed: " .. tostring(err))
      pcall(emu.displayMessage, "Romlens", "The recorder stopped: " .. tostring(err):match("[^\n]*"))
      pcall(finish)
      if frame_limit then emu.stop(3) end
    end
  end
end

local function on_dma(_, value)
  local s = emu.getState()
  local d = "D" .. string.pack("<I4I1I2", frame, value, value_of(s["ppu.scanline"])) .. dma_registers()
    .. string.pack("<I2I2BI2I4BI2", value_of(s["memoryManager.hClock"]) & 0xFFFF,
      value_of(s["ppu.vramAddress"]) & 0xFFFF, value_of(s["ppu.cgramAddress"]) & 0xFF,
      value_of(s["ppu.oamRamAddress"]) & 0xFFFF,
      value_of(s["memoryManager.registerHandlerB.wramPosition"]) & 0x1FFFF,
      value_of(s["cpu.k"]) & 0xFF, value_of(s["cpu.pc"]) & 0xFFFF)
  out:write(d)
  live_send(d)
end

local cpu = emu.cpuType.snes
emu.addMemoryCallback(guarded(on_ppu_write), emu.callbackType.write, 0x2100, 0x2133, cpu, M.snesRegister)
emu.addMemoryCallback(guarded(on_dma), emu.callbackType.write, 0x420B, 0x420B, cpu, M.snesRegister)

-- The sound side's events, as parallel arrays until the frame's end.
local a_kind, a_addr, a_value, a_cycle, a_clock, a_count = {}, {}, {}, {}, {}, 0
local spc = emu.cpuType.spc
local function spc_cycle() return emu.getCpuCycleCount(spc) end
local function on_port(address, value)
  a_count = a_count + 1
  a_kind[a_count], a_addr[a_count], a_value[a_count] = 0, address & 3, value
  a_cycle[a_count], a_clock[a_count] = spc_cycle(), get_clock()
end
local function on_spc_io(address, value)
  a_count = a_count + 1
  a_kind[a_count], a_addr[a_count], a_value[a_count] = 1, address & 0x0F, value
  a_cycle[a_count], a_clock[a_count] = spc_cycle(), 0
end
if audio then
  emu.addMemoryCallback(guarded(on_port), emu.callbackType.write, 0x2140, 0x2143, cpu, M.snesRegister)
  emu.addMemoryCallback(guarded(on_spc_io), emu.callbackType.write, 0x00F0, 0x00FF, spc, M.spcMemory)
end

local aram_previous = {}
local function dsp_registers()
  local parts = {}
  for i = 0, 31 do parts[i + 1] = string.pack("<I4", emu.read32(i * 4, M.spcDspRegisters)) end
  return table.concat(parts)
end

-- The frame's 'A' record, then its 'S' as a delta and, for a new live
-- connection, whole.
local function audio_records()
  local events = { "A", string.pack("<I4I4", frame, a_count) }
  for i = 1, a_count do
    events[i + 2] = string.pack("<BBBI8I8", a_kind[i], a_addr[i], a_value[i], a_cycle[i], a_clock[i])
  end
  a_count = 0
  local head = "S" .. string.pack("<I4I8", frame, spc_cycle())
  local delta, whole, changed = {}, {}, 0
  for b = 0, 255 do
    local bytes = read_block(M.spcRam, b * BLOCK, BLOCK)
    whole[b + 1] = string.pack("<I2", b) .. bytes
    if aram_previous[b] ~= bytes then
      aram_previous[b] = bytes
      changed = changed + 1
      delta[changed] = whole[b + 1]
    end
  end
  local dsp = dsp_registers()
  return table.concat(events),
    head .. string.pack("<I2", changed) .. table.concat(delta) .. dsp,
    head .. string.pack("<I2", 256) .. table.concat(whole) .. dsp
end

local previous = {}

local function write_frame()
  local state = emu.getState()
  local r = line_writes(state)
  if r then
    out:write(r)
    live_send(r)
  end
  if audio then
    local a, s, s_whole = audio_records()
    out:write(a, s)
    live_send(a)
    live_send(live and live.fresh and s_whole or s)
  end
  local changed = {}
  for i, k in ipairs(fields) do
    local v = value_of(state[k])
    if previous_values[i] ~= v then
      previous_values[i] = v
      changed[#changed + 1] = string.pack("<I2i8", i - 1, v)
    end
  end
  local regs = {}
  for i = 0, 0x33 do
    regs[i + 1] = last[i]
    regs[i + 53] = seen[i]
  end
  local registers = string.pack(REGISTER_FORMAT, table.unpack(regs, 1, 104)) .. dma_registers()
  local parts = { "F", string.pack("<I4I2", frame, #changed), table.concat(changed), registers }
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
  local bytes = table.concat(parts)
  out:write(bytes)
  if live and live.fresh then
    -- A new connection starts from nothing, so it gets every field and every
    -- block once; after that it gets what the file gets.
    live.fresh = false
    local full = { "F", string.pack("<I4I2", frame, #fields) }
    for i = 1, #fields do full[#full + 1] = string.pack("<I2i8", i - 1, previous_values[i] or 0) end
    full[#full + 1] = registers
    for r, region in ipairs(regions) do
      local n = (region[2] - 1) // BLOCK + 1
      full[#full + 1] = string.pack("<I2", n)
      for b = 0, n - 1 do full[#full + 1] = string.pack("<I2", b) .. previous[r][b] end
    end
    live_send(table.concat(full))
    if xlog_live then
      -- Everything so far, then deltas from here: taking one first sets the
      -- point the next delta counts from.
      emu.takeExecutionLogDelta()
      live_xlog(emu.getExecutionLog())
    end
  else
    live_send(bytes)
    if xlog_live and live and frame % 60 == 0 then live_xlog(emu.takeExecutionLogDelta()) end
  end
  frame = frame + 1
  if frame % 60 == 0 then out:flush() end
end

function finish()
  if finished then return end
  finished = true
  out:write("E", string.pack("<I4", frame))
  out:close()
  if live then
    live.sock:settimeout(1)
    -- The last delta, so the session's merged log is the whole log.
    if xlog_live then live_xlog(emu.takeExecutionLogDelta()) end
    live_send("E" .. string.pack("<I4", frame))
    live_close()
  end
  emu.log("Romlens recorder: " .. frame .. " frames to " .. out_path)
  if xlog then
    write_xlog()
    emu.stopExecutionLog()
    emu.log("Romlens recorder: execution log to " .. xlog_path)
  end
end

-- The stream describes the game loaded when the script started. Loading
-- another game leaves the script running, so notice and stop, rather than
-- send the new game's frames under the old one's header.
local started_sha1 = rom.fileSha1Hash or ""
local function game_changed()
  local now = emu.getRomInfo()
  local sha1 = (now and now.fileSha1Hash) or ""
  if sha1 == started_sha1 then return false end
  emu.log("Romlens recorder: the game changed to " .. ((now and now.name) or "another ROM")
    .. "; stopped. Run the script again to record this game.")
  live_close("the game changed")
  finish()
  return true
end

emu.addEventCallback(guarded(function()
  if finished then return end
  if frame % 120 == 0 then
    if game_changed() then return end
    live_try_connect()
  end
  write_frame()
  if xlog and frame % 3600 == 0 then write_xlog() end
  if frame_limit and frame >= frame_limit then
    finish()
    emu.stop(0)
  end
end), emu.eventType.endFrame)

emu.addEventCallback(guarded(function()
  -- The loaded state's registers were written before we could see them,
  -- and its clock is not this session's.
  reset_registers()
  ends, w_count = nil, 0
  out:write("L", string.pack("<I4", frame))
  live_send("L" .. string.pack("<I4", frame))
end), emu.eventType.stateLoaded)

emu.addEventCallback(guarded(finish), emu.eventType.scriptEnded)

emu.log("Romlens recorder: writing " .. out_path)
if xlog then
  emu.log("Romlens recorder: and an execution log, " .. xlog_path)
end
if has_socket then
  emu.log("Romlens recorder: will stream live to Romlens on port " .. LIVE_PORT .. " when it listens")
else
  emu.log("Romlens recorder: live streaming is off (turn on network access in the script settings)")
end

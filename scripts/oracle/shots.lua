-- Appended to the Romlens recorder by run.sh: Mesen's own screen at chosen
-- frames, beside the recording, so a frame Romlens composes can be checked
-- pixel for pixel (docs/22, P1). Development only; the screens stay in the
-- output folder on the developer's machine.
--
-- $ROMLENS_SHOT_DIR       where to write shot-<frame>.bin
-- $ROMLENS_SHOT_FRAMES    the frames, comma-separated
--
-- A dump is u16 width, u16 height, then width x height ARGB u32 pixels.
local shot_dir = os.getenv("ROMLENS_SHOT_DIR")
local want = {}
for n in string.gmatch(os.getenv("ROMLENS_SHOT_FRAMES") or "", "%d+") do want[tonumber(n)] = true end
local shot_frame = 0
emu.addEventCallback(function()
  if want[shot_frame] then
    local size = emu.getScreenSize()
    local px = emu.getScreenBuffer()
    local f = assert(io.open(shot_dir .. "/shot-" .. shot_frame .. ".bin", "wb"))
    f:write(string.pack("<I2I2", size.width, size.height))
    local parts = {}
    for i = 1, #px do parts[#parts + 1] = string.pack("<I4", px[i]) end
    f:write(table.concat(parts))
    f:close()
  end
  shot_frame = shot_frame + 1
end, emu.eventType.endFrame)

-- Apply only to the virtual keyboard created by ydotoold.
-- Physical Left Alt becomes Super; physical Left Windows becomes Alt.
hl.device({
  name = "ydotoold-virtual-device",
  kb_options = "compose:caps,shift:both_capslock_cancel,altwin:swap_lalt_lwin",
})

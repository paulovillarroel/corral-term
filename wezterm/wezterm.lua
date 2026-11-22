-- WezTerm for Corral: opens Ubuntu in WSL and lets through the images of the
-- Kitty graphics protocol, which herdr forwards from its panes.
-- Copy this file to C:\Users\<your user>\.wezterm.lua

local wezterm = require 'wezterm'
local config = wezterm.config_builder()

-- WezTerm 20240203 does not always detect the distro on its own, so declare it.
config.wsl_domains = {
  { name = 'WSL:Ubuntu-24.04', distribution = 'Ubuntu-24.04', default_cwd = '~' },
}
config.default_domain = 'WSL:Ubuntu-24.04'
config.enable_kitty_graphics = true

-- herdr draws its own tabs and panes; WezTerm only provides the window.
config.hide_tab_bar_if_only_one_tab = true
config.window_decorations = 'INTEGRATED_BUTTONS|RESIZE'
config.window_padding = { left = 6, right = 6, top = 6, bottom = 2 }
config.colors = { background = '#101010' }

config.font_size = 11
config.initial_cols = 200
config.initial_rows = 52

-- No bell and no close prompts: herdr keeps everything even if you close the window.
config.audible_bell = 'Disabled'
config.window_close_confirmation = 'NeverPrompt'

return config

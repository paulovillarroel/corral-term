-- WezTerm para Corral: abre Ubuntu en WSL y deja pasar las imágenes del
-- protocolo de Kitty, que herdr reenvía desde sus paneles.
-- Copia este archivo a C:\Users\<tu usuario>\.wezterm.lua

local wezterm = require 'wezterm'
local config = wezterm.config_builder()

-- WezTerm 20240203 does not always detect the distro on its own, so declare it.
config.wsl_domains = {
  { name = 'WSL:Ubuntu-24.04', distribution = 'Ubuntu-24.04', default_cwd = '~' },
}
config.default_domain = 'WSL:Ubuntu-24.04'
config.enable_kitty_graphics = true

-- herdr dibuja sus propias pestañas y paneles; WezTerm solo pone la ventana.
config.hide_tab_bar_if_only_one_tab = true
config.window_decorations = 'INTEGRATED_BUTTONS|RESIZE'
config.window_padding = { left = 6, right = 6, top = 6, bottom = 2 }
config.colors = { background = '#101010' }

config.font_size = 11
config.initial_cols = 200
config.initial_rows = 52

-- Sin sonido ni avisos de cierre: herdr mantiene todo aunque cierres la ventana.
config.audible_bell = 'Disabled'
config.window_close_confirmation = 'NeverPrompt'

return config

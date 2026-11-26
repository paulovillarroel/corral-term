# profile.R: how Corral's R consoles load Corral, whatever profile the project has.
#
# The consoles Corral starts (Ark, or plain R) set R_PROFILE_USER to this file
# (console::r_profile_env in rs/src/console.rs), so R reads it instead of a user
# profile. It does what R would have done, and then loads Corral:
#
#   1. Keeps your R library's path in CORRAL_R_LIBS_USER before anything changes it:
#      a project with renv points R_LIBS_USER at its own library, and Corral's
#      packages (jsonlite, httpgd, nanoparquet, duckdb) are in yours.
#   2. Reads the profile R would have read: yours, if you set R_PROFILE_USER
#      (CORRAL_R_PROFILE_USER keeps it), else the project's .Rprofile (where renv
#      activates its library), else ~/.Rprofile.
#   3. Loads corral.R (CORRAL_R_FILE), unless that profile already did, as the line
#      the installer puts in ~/.Rprofile does, even when it points at another copy.
#
# A few file checks: it adds nothing you could time to R's start.
local({
  Sys.setenv(CORRAL_R_LIBS_USER = Sys.getenv("R_LIBS_USER"))
  own <- Sys.getenv("CORRAL_R_PROFILE_USER")
  profile <- if (nzchar(own)) own else if (file.exists(".Rprofile")) ".Rprofile" else "~/.Rprofile"
  profile <- path.expand(profile)
  # As R does with a user profile: in the global environment, and an error in it
  # doesn't stop R from starting.
  if (file.exists(profile)) try(source(profile, local = globalenv()))
  corral <- Sys.getenv("CORRAL_R_FILE")
  # Already loaded by that profile: the flag (this version on), or Corral's task callback,
  # which every version registers as "corral" (a ~/.Rprofile pointing at an older copy).
  loaded <- isTRUE(getOption("corral.loaded")) || "corral" %in% getTaskCallbackNames()
  if (!loaded && interactive() && nzchar(corral) && file.exists(corral)) {
    try(source(corral, local = globalenv()))
  }
})

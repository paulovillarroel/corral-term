# nanoparquet against arrow, writing tables of different shapes: why View() tries
# nanoparquet first (write_table() in r/corral.R).
#
#   Rscript bench/writers.R [runs]      (10 runs by default)
#
# Times are the median of the runs, with the fastest and slowest in brackets.
# Each write starts after gc(), with the previous table gone: nanoparquet
# allocates inside R and arrow outside it, so R's garbage collector would
# otherwise land in nanoparquet's time, by an amount that depends on whatever
# else is in memory.
# Memory isn't measured here; bench/repeat.R follows R's memory over a long session.

args <- commandArgs(trailingOnly = TRUE)
runs <- if (length(args) >= 1) as.integer(args[[1]]) else 10L
for (pkg in c("arrow", "nanoparquet")) {
  if (!requireNamespace(pkg, quietly = TRUE)) stop(pkg, " isn't installed")
}

set.seed(2)
shapes <- list(
  "2,000,000 rows, 50 numeric columns" = function() as.data.frame(matrix(rnorm(2e6 * 50), ncol = 50)),
  "5,000,000 rows of text" = function() {
    n <- 5e6
    data.frame(id = sprintf("ID%09d", sample.int(1e9, n)), email = paste0("u", seq_len(n), "@example.org"),
               city = sample(c("Santiago", "Valparaíso", "Concepción", "Temuco"), n, TRUE),
               note = sprintf("note %d", sample.int(1e6, n, TRUE)), x = runif(n))
  },
  "5,000,000 rows of factors, dates and NA" = function() {
    n <- 5e6
    d <- data.frame(f = factor(sample(letters, n, TRUE)),
                    t = as.POSIXct("2024-01-01", tz = "UTC") + runif(n, 0, 3e7),
                    d = as.Date("2020-01-01") + sample(0:3000, n, TRUE),
                    b = sample(c(TRUE, FALSE, NA), n, TRUE), i = sample(c(1:100, NA), n, TRUE), y = rnorm(n))
    d$y[sample.int(n, n / 10)] <- NA
    d
  },
  "32,000 rows (mtcars)" = function() mtcars[rep(1:32, 1000), ]
)
writers <- list(
  nanoparquet = function(x, f) nanoparquet::write_parquet(x, f),
  arrow = function(x, f) arrow::write_parquet(x, f)
)

elapsed <- function(expr) {
  t <- proc.time()[["elapsed"]]
  force(expr)
  proc.time()[["elapsed"]] - t
}
secs <- function(s) sprintf("%.2f s", s)

cat(sprintf("R %s · nanoparquet %s · arrow %s · %d runs\n\n", getRversion(),
            packageVersion("nanoparquet"), packageVersion("arrow"), runs))
for (s in names(shapes)) {
  x <- shapes[[s]]()
  cat(s, "\n")
  for (w in names(writers)) {
    t <- numeric(runs)
    for (i in seq_len(runs)) {
      f <- tempfile("corral-bench-", fileext = ".parquet")
      invisible(gc())
      t[i] <- elapsed(writers[[w]](x, f))
      size <- file.size(f)
      unlink(f)
    }
    cat(sprintf("  %-12s %s [%s–%s]  %.0f MB\n", w, secs(stats::median(t)), secs(min(t)), secs(max(t)), size / 1e6))
  }
  cat("\n")
  rm(x)
  invisible(gc())
}

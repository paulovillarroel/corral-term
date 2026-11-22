# View() again and again in one long session: how steady the time is, and how
# R's memory grows, with nanoparquet or with arrow.
#
#   Rscript bench/repeat.R nanoparquet [times]
#   Rscript bench/repeat.R arrow [times]       (20 times by default)
#
# Run it once per package: each run is its own R process, so their memory doesn't
# mix. The session holds the 10,000,000-row sales table of bench/view.R and 5
# million other strings, like a session in the middle of a day's work. Each time
# it writes the table as View() does and deletes the file; the memory is R's
# resident memory (VmRSS), after each write.

args <- commandArgs(trailingOnly = TRUE)
w <- if (length(args) >= 1) args[[1]] else "nanoparquet"
times <- if (length(args) >= 2) as.integer(args[[2]]) else 20L
if (!w %in% c("nanoparquet", "arrow")) stop("say nanoparquet or arrow")
if (!requireNamespace(w, quietly = TRUE)) stop(w, " isn't installed")

rss <- function() {
  line <- grep("^VmRSS:", readLines("/proc/self/status"), value = TRUE)
  as.numeric(gsub("[^0-9]", "", line)) / 1024
}

set.seed(1)
n <- 1e7
products <- c("Laptop", "Monitor", "Dock", "Headset", "Keyboard", "Webcam", "Mouse", "Tablet")
sales <- data.frame(
  order_id = seq_len(n),
  date = as.Date("2024-01-01") + sample(0:729, n, TRUE),
  region = sample(c("North", "South", "East", "West"), n, TRUE),
  product = sample(products, n, TRUE),
  units = sample(1:12, n, TRUE),
  price = round(runif(n, 20, 1500), 2),
  stringsAsFactors = FALSE
)
sales$revenue <- round(sales$units * sales$price, 2)
other <- data.frame(id = sprintf("ID%09d", sample.int(1e9, 5e6)), note = sprintf("note %d", sample.int(1e6, 5e6, TRUE)))
write <- if (w == "arrow") function(f) arrow::write_parquet(sales, f) else function(f) nanoparquet::write_parquet(sales, f)

cat(sprintf("%s %s · %d times · R's memory before: %.0f MB\n", w, packageVersion(w), times, rss()))
t <- mem <- numeric(times)
for (i in seq_len(times)) {
  f <- tempfile("corral-bench-", fileext = ".parquet")
  s <- proc.time()[["elapsed"]]
  write(f)
  t[i] <- proc.time()[["elapsed"]] - s
  mem[i] <- rss()
  unlink(f)
  if (i %% 5 == 0) cat(sprintf("  after %2d: %.2f s, %.0f MB\n", i, t[i], mem[i]))
}
cat(sprintf("time: median %.2f s, fastest %.2f s, slowest %.2f s\n", stats::median(t), min(t), max(t)))
cat(sprintf("R's memory: %.0f MB after the first, %.0f MB after the last, %.0f MB at most\n", mem[1], mem[times], max(mem)))

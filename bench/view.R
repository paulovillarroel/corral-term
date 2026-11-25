# How long View(df) takes for a big table, from the call to the first screen.
#
#   Rscript bench/view.R [rows] [runs]      (10,000,000 rows and 10 runs by default)
#
# View() does two things, and this measures both:
#   1. R writes the table to a Parquet file: nanoparquet, else arrow, else CSV.
#      These are the same calls as write_table() in r/corral.R.
#   2. The viewer (corral _table, rs/src/table.rs) starts DuckDB and runs three
#      queries before it draws: the columns, the row count and the first page of
#      200 rows. Then a page from the middle and a sort of every row, as you would
#      scroll or sort. `corral _table --bench` times these with the viewer's own
#      code, one DuckDB per run, as each View() opens its own viewer.
# Opening the herdr popup and drawing it aren't counted: a few milliseconds.
# Times are the median of the runs, with the fastest and slowest in brackets.

args <- commandArgs(trailingOnly = TRUE)
n <- if (length(args) >= 1) as.numeric(args[[1]]) else 1e7
runs <- if (length(args) >= 2) as.integer(args[[2]]) else 10L
if (!nzchar(Sys.which("duckdb"))) stop("duckdb isn't on the PATH")
if (!nzchar(Sys.which("corral"))) stop("corral isn't on the PATH")

# The table in the site's screenshots: sales, 7 columns.
set.seed(1)
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

elapsed <- function(expr) {
  t <- proc.time()[["elapsed"]]
  force(expr)
  proc.time()[["elapsed"]] - t
}
ms <- function(s) sprintf("%.0f ms", 1000 * s)
secs <- function(s) if (s < 1) ms(s) else sprintf("%.1f s", s)
summary_of <- function(t, unit = secs) {
  if (length(t) == 1) return(unit(t))
  sprintf("%s [%s–%s]", unit(stats::median(t)), unit(min(t)), unit(max(t)))
}
viewer <- function(f) {
  out <- system2("corral", c("_table", "--bench", shQuote(f), "revenue"), stdout = TRUE)
  if (!is.null(attr(out, "status"))) stop("corral _table --bench failed on ", f)
  jsonlite::fromJSON(paste(out, collapse = ""))
}

writers <- list(
  nanoparquet = function(f) nanoparquet::write_parquet(sales, f),
  arrow = function(f) arrow::write_parquet(sales, f),
  csv = function(f) utils::write.csv(sales, f, row.names = FALSE)
)

cat(sprintf("%s rows, %d columns · R %s · DuckDB %s · %d runs\n\n",
            format(n, big.mark = ",", scientific = FALSE), ncol(sales), getRversion(),
            sub(" .*", "", system2("duckdb", "--version", stdout = TRUE)), runs))

for (w in names(writers)) {
  if (w != "csv" && !requireNamespace(w, quietly = TRUE)) {
    cat(sprintf("%s: not installed\n\n", w))
    next
  }
  # CSV is the fallback and slow: three runs are enough to show it.
  k <- if (w == "csv") min(runs, 3L) else runs
  f <- tempfile("corral-bench-", fileext = if (w == "csv") ".csv" else ".parquet")
  write <- screen <- middle <- sort <- numeric(k)
  for (i in seq_len(k)) {
    unlink(f)
    write[i] <- elapsed(writers[[w]](f))
    t <- viewer(f)
    screen[i] <- t$first
    middle[i] <- t$middle
    sort[i] <- t$sort
  }
  version <- if (w == "csv") "utils::write.csv" else paste(w, packageVersion(w))
  cat(sprintf("%s · file %.0f MB\n", version, file.size(f) / 1e6))
  cat(sprintf("  writing the file        %s\n", summary_of(write)))
  cat(sprintf("  first screen            %s\n", summary_of(screen)))
  cat(sprintf("  View() in all           %s\n", summary_of(write + screen)))
  cat(sprintf("  a page from the middle  %s\n", summary_of(middle)))
  cat(sprintf("  sort every row          %s\n\n", summary_of(sort)))
  unlink(f)
}

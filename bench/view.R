# How long View(df) takes for a big table, from the call to the first screen.
#
#   Rscript bench/view.R [rows] [runs]      (10,000,000 rows and 10 runs by default)
#
# View() does two things, and this measures both:
#   1. R writes the table to a Parquet file: nanoparquet, else arrow, else CSV.
#      These are the same calls as write_table() in r/corral.R.
#   2. The viewer (corral _table, rs/src/table.rs) runs three DuckDB queries before
#      it draws: the columns, the row count and the first page of 200 rows. Each is
#      one `duckdb -json -c` call, as in the viewer. Then a page from the middle and
#      a sort of every row, as you would scroll or sort.
# Opening the herdr popup and drawing it aren't counted: a few milliseconds.
# Times are the median of the runs, with the fastest and slowest in brackets.

args <- commandArgs(trailingOnly = TRUE)
n <- if (length(args) >= 1) as.numeric(args[[1]]) else 1e7
runs <- if (length(args) >= 2) as.integer(args[[2]]) else 10L
if (!nzchar(Sys.which("duckdb"))) stop("duckdb isn't on the PATH")

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
duck <- function(sql) {
  out <- system2("duckdb", c("-json", "-c", shQuote(sql)), stdout = TRUE)
  if (!is.null(attr(out, "status"))) stop("DuckDB failed: ", sql)
  invisible(out)
}
source_of <- function(f) {
  if (grepl("\\.parquet$", f)) sprintf("read_parquet('%s')", f) else sprintf("read_csv_auto('%s')", f)
}
first_screen <- function(f) {
  src <- source_of(f)
  elapsed({
    duck(sprintf("DESCRIBE SELECT * FROM %s", src))
    duck(sprintf("SELECT count(*) AS n FROM %s", src))
    duck(sprintf("SELECT * FROM %s LIMIT 200 OFFSET 0", src))
  })
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
    screen[i] <- first_screen(f)
    middle[i] <- elapsed(duck(sprintf("SELECT * FROM %s LIMIT 200 OFFSET %d", source_of(f), n %/% 2)))
    sort[i] <- elapsed(duck(sprintf("SELECT * FROM %s ORDER BY revenue DESC LIMIT 200", source_of(f))))
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

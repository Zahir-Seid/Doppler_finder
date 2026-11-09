# Weather source: Open-Meteo

FR-DA-03 needs measured temperature, pressure, humidity and wind for the
propagation path, with an explicitly flagged standard-atmosphere fallback.
`acoustics::weather` (ACO-01) defines the normalized payload and the
window averaging; `adsb::weather` (ACO-02) is the fetch adapter that
produces that payload from Open-Meteo.

## Endpoints

| Constant | Endpoint | Coverage |
| --- | --- | --- |
| `ARCHIVE_API_URL` | `archive-api.open-meteo.com/v1/archive` | ERA5 reanalysis, roughly 1940 to ~5 days ago |
| `HIST_FORECAST_API_URL` | `historical-forecast-api.open-meteo.com/v1/forecast` | Operational forecast archive, roughly the last two years |
| `FORECAST_API_URL` | `api.open-meteo.com/v1/forecast` | Recent forecast (not used by the archive path) |

`WeatherArchive` selects between the archive and historical-forecast
products; recordings made in the past almost always want `Archive`.

## Request shape

`open_meteo_url` requests hourly `temperature_2m`,
`relative_humidity_2m`, `surface_pressure`, `wind_speed_10m` and
`wind_direction_10m` for one location over the UTC dates the window
touches, with `wind_speed_unit=ms`, `timeformat=unixtime` and
`timezone=GMT`. The API works on whole dates, so returned samples can fall
outside the requested window; `acoustics::weather::conditions_in_window`
does the exact windowing downstream.

## Response handling

- The response is a single-location object (an array is accepted and the
  first element used, so a hand-built multi-point request cannot silently
  return the wrong site's weather).
- Unit conversions: °C → K, hPa → Pa, % → fraction.
- `wind_direction_10m` is the meteorological *from* direction; the payload
  stores toward-components per the Q-10 convention
  (`wind_east = -speed·sin θ`, `wind_north = -speed·cos θ`).
- Rows with any missing or non-finite field are skipped, never
  fabricated. A response with no usable rows yields an empty `samples`
  array, which downstream resolves to the flagged standard atmosphere.
- Structure-level problems (non-JSON, missing `hourly` block or `time`
  axis) are `AdsbError::Parse`.

## Transport guard

`fetch_weather_payload` goes through `adsb::http::HttpPolicy::fetch`, so
the request is HTTPS-only, host-allow-listed, size-capped and retried like
every other archive fetch. `OPEN_METEO_HOSTS` lists the three Open-Meteo
hosts to add to the policy's allow-list; a host outside the list is
rejected before any request is sent.

## Tests and licence

`crates/adsb/tests/aco_02.rs` replays an in-file recorded response through
a scripted `HttpTransport` — no test opens a socket and no live data is
committed. The payload is verified end-to-end by feeding it to
`acoustics::weather::parse_weather` and `atmosphere_for_window` (adsb's
test-only dev-dependency on `acoustics`; the library crates stay
dependency-free of each other).

Open-Meteo data is published under CC BY 4.0 for the compiled product;
the underlying ERA5/model data keeps its own terms. See
`docs/third_party.md` for the inventory entry and the redistribution
caveat.

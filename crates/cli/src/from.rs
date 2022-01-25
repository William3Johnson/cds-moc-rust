
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::str::{self, FromStr};
use std::num::ParseFloatError;
use std::path::PathBuf;
use std::error::Error;
use std::ops::Range;

use structopt::StructOpt;

use healpix::nested::Layer;

use moclib::qty::{MocQty, Hpx, Time};
use moclib::elem::valuedcell::valued_cells_to_moc_with_opt;
use moclib::elemset::range::HpxRanges;
use moclib::moc::RangeMOCIntoIterator;
use moclib::moc::range::RangeMOC;
use moclib::moc2d::{
  RangeMOC2IntoIterator,
  range::RangeMOC2
};

use super::InputTime;
use super::output::OutputFormat;

const HALF_PI: f64 = 0.5 * std::f64::consts::PI;
const PI: f64 = std::f64::consts::PI;
const TWICE_PI: f64 = 2.0 * std::f64::consts::PI;

#[derive(Debug)]
pub struct Vertices {
  // (ra0,dec0),(ra1,dec1),...,(ran,decn)
  list: Vec<(f64, f64)>
}

impl FromStr for Vertices {
  type Err = ParseFloatError;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    let list: Vec<f64> = s
      .replace("(", "")
      .replace(")", "")
      .split(',')
      .map(|t| str::parse::<f64>(t.trim()))
      .collect::<Result<Vec<f64>, _>>()?;
    Ok(
      Vertices {
        list: list.iter().step_by(2).zip(list.iter().skip(1).step_by(2))
          .map(|(lon, lat)| (*lon, *lat))
          .collect()
      }
    )
  }
}

#[derive(StructOpt, Debug)]
pub enum From {
  #[structopt(name = "cone")]
  /// Create a Spatial MOC from the given cone
  Cone {
    /// Depth of the created MOC, in `[0, 29]`.
    depth: u8,
    /// Longitude of the cone center (in degrees)
    lon_deg: f64,
    /// Latitude of the cone center (in degrees)
    lat_deg: f64,
    /// Radius of the cone (in degrees)
    r_deg: f64,
    #[structopt(subcommand)]
    out: OutputFormat
    // add option: inside / overallaping / partially_in / centers_in 
  },
  #[structopt(name = "ring")]
  /// Create a Spatial MOC from the given ring
  Ring {
    /// Depth of the created MOC, in `[0, 29]`.
    depth: u8,
    /// Longitude of the ring center (in degrees)
    lon_deg: f64,
    /// Latitude of the ring center (in degrees)
    lat_deg: f64,
    /// Internal radius of the ring (in degrees)
    r_int_deg: f64,
    /// External radius of the ring (in degrees)
    r_ext_deg: f64,
    #[structopt(subcommand)]
    out: OutputFormat
  },
  #[structopt(name = "ellipse")]
  /// Create a Spatial MOC from the given elliptical cone
  EllipticalCone {
    /// Depth of the created MOC, in `[0, 29]`.
    depth: u8,
    /// Longitude of the elliptical cone center (in degrees)
    lon_deg: f64,
    /// Latitude of the elliptical cone center (in degrees)
    lat_deg: f64,
    /// Elliptical cone semi-major axis (in degrees)
    a_deg: f64,
    /// Elliptical cone semi-minor axis (in degrees)
    b_deg: f64,
    /// Elliptical cone position angle (in degrees)
    pa_deg: f64,
    #[structopt(subcommand)]
    out: OutputFormat
  },
  #[structopt(name = "zone")]
  /// Create a Spatial MOC from the given zone
  Zone {
    /// Depth of the created MOC, in `[0, 29]`.
    depth: u8,
    /// Longitude min, in degrees
    lon_deg_min: f64,
    /// Latitude min, in degrees
    lat_deg_min: f64,
    /// Longitude max, in degrees
    lon_deg_max: f64,
    /// Latitude max, in degrees
    lat_deg_max: f64,
    #[structopt(subcommand)]
    out: OutputFormat
  },
  #[structopt(name = "box")]
  /// Create a Spatial MOC from the given box
  Box { // transform into a polygon!
    /// Depth of the created MOC, in `[0, 29]`.
    depth: u8,
    /// Longitude of the box center, in degrees
    lon_deg: f64,
    /// Latitude of the box center, in degrees
    lat_deg: f64,
    /// Semi-major axis of the box, in degrees
    a_deg: f64,
    /// Semi-minor axis of the box, in degrees
    b_deg: f64,
    /// Position angle of the box, in degrees
    pa_deg: f64,
    #[structopt(subcommand)]
    out: OutputFormat
  },
  #[structopt(name = "polygon")]
  /// Create a Spatial MOC from the given polygon
  Polygon {
    /// Depth of the created MOC, in `[0, 29]`.
    depth: u8,
    /// List of vertices: "(lon,lat),(lon,lat),...,(lon,lat)" in degrees
    vertices_deg: Vertices, // (ra0,dec0),(ra1,dec1),...,(ran,decn)
    #[structopt(short = "c", long)]
    /// Gravity center of the polygon out of the polygon (in by default)
    complement: bool,
    #[structopt(subcommand)]
    out: OutputFormat
  },
  #[structopt(name = "pos")]
  /// Create a Spatial MOC from a list of positions in decimal degrees (one pair per line, longitude first, then latitude).
  Positions {
    /// Depth of the created MOC, in `[0, 29]`.
    depth: u8,
    #[structopt(parse(from_os_str))]
    /// The input file, use '-' for stdin
    input: PathBuf,
    #[structopt(short = "s", long = "separator", default_value = " ")]
    /// Separator between both coordinates (default = ' ')
    separator: String,
    #[structopt(subcommand)]
    out: OutputFormat
  },
  #[structopt(name = "vuniq")]
  /// Create a Spatial MOC from a list of (non-overlapping) uniq cells associated with values (uniq first, then value),
  /// i.e. from a multi-resolution map, putting completeness constraints.
  ValuedCells {
    /// Depth of the created MOC, in `[0, 29]`. Must be >= largest input cells depth.
    depth: u8,
    #[structopt(short = "d", long = "density")]
    /// Input values are densities, i.e. they are not proportional to the area of their associated cells.
    density: bool,
    #[structopt(short = "f", long = "from", default_value = "0")]
    /// Cumulative value at which we start putting cells in he MOC.
    from_threshold: String,
    #[structopt(short = "t", long = "to", default_value = "1")] // Valid for a proba (sum = 1 on the all sky)
    /// Cumulative value at which we stop putting cells in the MOC.
    to_threshold: String,
    #[structopt(short = "a", long = "asc")]
    /// Compute cumulative value from ascending density values instead of descending.
    asc: bool,
    #[structopt(short = "s", long = "not-strict")]
    /// Cells overlapping with the upper or the lower cumulative bounds are not rejected.
    not_strict: bool,
    #[structopt(short = "p", long = "no-split")]
    /// Split recursively the cells overlapping the upper or the lower cumulative bounds.
    split: bool,
    #[structopt(short = "r", long = "rev-descent")]
    /// Perform the recursive descent from the highest to the lowest sub-cell, only with option 'split' 
    /// (set both flags to be compatibile with Aladin) 
    revese_recursive_descent: bool,
    #[structopt(parse(from_os_str))]
    /// The input file, use '-' for stdin
    input: PathBuf,
    #[structopt(short = "s", long = "separator", default_value = " ")]
    /// Separator between both coordinates (default = ' ')
    separator: String,
    #[structopt(subcommand)]
    out: OutputFormat
  },
  #[structopt(name = "timestamp")]
  /// Create a Time MOC from a list of timestamp (one per line).
  Timestamp {
    /// Depth of the created MOC, in `[0, 61]`.
    depth: u8,
    #[structopt(long = "time-type", default_value = "jd")]
    /// Time type: 'jd' (julian date), 'mjd' (modified julian date), 'usec' (microsec since JD=0), 
    /// 'isorfc' (Gregorian date-time, Rfc3339, WARNING: no conversion to TCB),
    /// or 'isosimple' (Gregorian date, 'YYYY-MM-DDTHH:MM:SS' WARNING: no conversion to TCB)
    time: InputTime,
    #[structopt(parse(from_os_str))]
    /// The input file, use '-' for stdin
    input: PathBuf,
    #[structopt(subcommand)]
    out: OutputFormat
  },
  #[structopt(name = "timerange")]
  /// Create a Time MOC from a list of time range (one range per line, lower bound first, then upper bound).
  Timerange {
    /// Depth of the created MOC, in `[0, 61]`.
    depth: u8,
    #[structopt(long = "time-type", default_value = "jd")]
    /// Time type: 'jd' (julian date), 'mjd' (modified julian date), 'usec' (microsec since JD=0), 
    /// 'isorfc' (Gregorian date-time, Rfc3339, WARNING: no conversion to TCB),
    /// or 'isosimple' (Gregorian date, 'YYYY-MM-DDTHH:MM:SS' WARNING: no conversion to TCB)
    time: InputTime,
    #[structopt(parse(from_os_str))]
    /// The input file, use '-' for stdin
    input: PathBuf,
    #[structopt(short = "s", long = "separator", default_value = " ")]
    /// Separator between time lower and upper bounds (default = ' ')
    separator: String,
    #[structopt(subcommand)]
    out: OutputFormat
  },
  #[structopt(name = "timestamppos")]
  /// Create a Space-Time MOC from a list of timestamp and positions in decimal degrees 
  /// (timestamp first, then longitude, then latitude)..
  TimestampPos {
    /// Depth on the time, in `[0, 61]`.
    tdepth: u8,
    /// Depth on the position, in `[0, 29]`.
    sdepth: u8,
    #[structopt(long = "time-type", default_value = "jd")]
    /// Time type: 'jd' (julian date), 'mjd' (modified julian date), 'usec' (microsec since JD=0), 
    /// 'isorfc' (Gregorian date-time, Rfc3339, WARNING: no conversion to TCB),
    /// or 'isosimple' (Gregorian date, 'YYYY-MM-DDTHH:MM:SS' WARNING: no conversion to TCB)
    time: InputTime,
    #[structopt(parse(from_os_str))]
    /// The input file, use '-' for stdin
    input: PathBuf,
    #[structopt(short = "s", long = "separator", default_value = " ")]
    /// Separator between time lower and upper bounds (default = ' ')
    separator: String,
    #[structopt(subcommand)]
    out: OutputFormat
  },
  #[structopt(name = "timerangepos")]
  /// Create a Space-Time MOC from a list of time range and positions in decimal degrees 
  /// (tmin first, then tmax, then longitude, and latitude)..
  TimerangePos {
    /// Depth on the time, in `[0, 61]`.
    tdepth: u8,
    /// Depth on the position, in `[0, 29]`.
    sdepth: u8,
    #[structopt(long = "time-type", default_value = "jd")]
    /// Time type: 'jd' (julian date), 'mjd' (modified julian date), 'usec' (microsec since JD=0), 
    /// 'isorfc' (Gregorian date-time, Rfc3339, WARNING: no conversion to TCB),
    /// or 'isosimple' (Gregorian date, 'YYYY-MM-DDTHH:MM:SS' WARNING: no conversion to TCB)
    time: InputTime,
    #[structopt(parse(from_os_str))]
    /// The input file, use '-' for stdin
    input: PathBuf,
    #[structopt(short = "s", long = "separator", default_value = " ")]
    /// Separator between time lower and upper bounds (default = ' ')
    separator: String,
    #[structopt(subcommand)]
    out: OutputFormat
  },
}

impl From {
  pub fn exec(self) -> Result<(), Box<dyn Error>> {
    // println!("From {:?}", from);
    match self {
      From::Cone {
        depth,
        lon_deg,
        lat_deg,
        r_deg,
        out
      } => {
        let lon = lon_deg2rad(lon_deg)?;
        let lat = lat_deg2rad(lat_deg)?;
        let r = r_deg.to_radians();
        if r <= 0.0 || PI <= r {
          Err(String::from("Radius must be in ]0, pi[").into())
        } else {
          let moc: RangeMOC<u64, Hpx<u64>> = RangeMOC::from_cone(lon, lat, r, depth, 2);
          out.write_smoc_possibly_auto_converting_from_u64(moc.into_range_moc_iter())
        }
      },
      From::Ring {
        depth,
        lon_deg,
        lat_deg,
        r_int_deg,
        r_ext_deg,
        out
      } => {
        let lon = lon_deg2rad(lon_deg)?;
        let lat = lat_deg2rad(lat_deg)?;
        let r_int = r_int_deg.to_radians();
        let r_ext = r_ext_deg.to_radians();
        if r_int <= 0.0 || PI <= r_int {
          Err(String::from("Internal radius must be in ]0, pi[").into())
        } else if r_ext <= 0.0 || PI <= r_ext {
          Err(String::from("External radius must be in ]0, pi[").into())
        } else if r_ext <= r_int {
          Err(String::from("External radius must be larger than the internal radius").into())
        } else {
          let moc: RangeMOC<u64, Hpx<u64>> = RangeMOC::from_ring(lon, lat, r_int, r_ext, depth, 2);
          out.write_smoc_possibly_auto_converting_from_u64(moc.into_range_moc_iter())
        }
      },
      From::EllipticalCone {
        depth,
        lon_deg,
        lat_deg,
        a_deg,
        b_deg,
        pa_deg,
        out
      } => {
        let lon = lon_deg2rad(lon_deg)?;
        let lat = lat_deg2rad(lat_deg)?;
        let a = a_deg.to_radians();
        let b = b_deg.to_radians();
        let pa = pa_deg.to_radians();
        if a <= 0.0 || HALF_PI <= a {
          Err(String::from("Semi-major axis must be in ]0, pi/2]").into())
        } else if b <= 0.0 || a <= b {
          Err(String::from("Semi-minor axis must be in ]0, a[").into())
        } else if pa <= 0.0 || HALF_PI <= pa {
          Err(String::from("Position angle must be in [0, pi[").into())
        } else {
          let moc: RangeMOC<u64, Hpx<u64>> = RangeMOC::from_elliptical_cone(lon, lat, a, b, pa, depth, 2);
          out.write_smoc_possibly_auto_converting_from_u64(moc.into_range_moc_iter())
        }
      },
      From::Zone {
        depth,
        lon_deg_min,
        lat_deg_min,
        lon_deg_max,
        lat_deg_max,
        out
      } => {
        let lon_min = lon_deg2rad(lon_deg_min)?;
        let lat_min = lat_deg2rad(lat_deg_min)?;
        let lon_max = lon_deg2rad(lon_deg_max)?;
        let lat_max = lat_deg2rad(lat_deg_max)?;
        let moc: RangeMOC<u64, Hpx<u64>> = RangeMOC::from_zone(lon_min, lat_min, lon_max, lat_max, depth);
        out.write_smoc_possibly_auto_converting_from_u64(moc.into_range_moc_iter())
      },
      From::Box {
        depth,
        lon_deg,
        lat_deg,
        a_deg,
        b_deg,
        pa_deg,
        out
      } => {
        let lon = lon_deg2rad(lon_deg)?;
        let lat = lat_deg2rad(lat_deg)?;
        let a = a_deg.to_radians();
        let b = b_deg.to_radians();
        let pa = pa_deg.to_radians();
        if a <= 0.0 || HALF_PI <= a {
          Err(String::from("Semi-major axis must be in ]0, pi/2]").into())
        } else if b <= 0.0 || a <= b {
          Err(String::from("Semi-minor axis must be in ]0, a[").into())
        } else if pa < 0.0 || PI <= pa {
          Err(String::from("Position angle must be in [0, pi[").into())
        } else {
          let moc: RangeMOC<u64, Hpx<u64>> = RangeMOC::from_box(lon, lat, a, b, pa, depth);
          out.write_smoc_possibly_auto_converting_from_u64(moc.into_range_moc_iter())
        }
      },
      From::Polygon {
        depth,
        vertices_deg,
        complement,
        out
      } => {
        let vertices = vertices_deg.list.iter()
          .map(|(lon_deg, lat_deg)| {
            let lon = lon_deg2rad(*lon_deg)?;
            let lat = lat_deg2rad(*lat_deg)?;
            Ok((lon, lat))
          }).collect::<Result<Vec<(f64, f64)>, Box<dyn Error>>>()?;
        let moc: RangeMOC<u64, Hpx<u64>> = RangeMOC::from_polygon(&vertices, complement, depth);
        out.write_smoc_possibly_auto_converting_from_u64(moc.into_range_moc_iter())
      },
      From::Positions {
        depth,
        input,
        separator,
        out
      } => {
        fn line2coos(separator: &str, line: std::io::Result<String>) -> Result<(f64, f64), Box<dyn Error>> {
          let line = line?;
          let (lon_deg, lat_deg) = line.trim()
            .split_once(separator)
            .ok_or_else(|| String::from("split on space failed."))?;
          let lon_deg = lon_deg.parse::<f64>()?;
          let lat_deg = lat_deg.parse::<f64>()?;
          let lon = lon_deg2rad(lon_deg)?;
          let lat = lat_deg2rad(lat_deg)?;
          Ok((lon, lat))
        }
        let line2pos = move |line: std::io::Result<String>| {
          match line2coos(&separator, line) {
            Ok(lonlat) => Some(lonlat),
            Err(e) => {
              eprintln!("Error reading or parsing line: {:?}", e);
              None
            }
          }
        };
        let moc: RangeMOC<u64, Hpx<u64>> = if input == PathBuf::from(r"-") {
          let stdin = std::io::stdin();
          RangeMOC::from_coos(depth, stdin.lock().lines().filter_map(line2pos), None)
        } else {
          let f = File::open(input)?;
          let reader = BufReader::new(f);
          RangeMOC::from_coos(depth, reader.lines().filter_map(line2pos), None)
        };
        out.write_smoc_possibly_auto_converting_from_u64(moc.into_range_moc_iter())
      },
      From::ValuedCells {
        depth,
        density,
        from_threshold,
        to_threshold,
        asc,
        not_strict,
        split,
        revese_recursive_descent,
        input,
        separator,
        out
      } => {
        let from_threshold = from_threshold.parse::<f64>()?;
        let to_threshold = to_threshold.parse::<f64>()?;
        fn line2uvd_from_val(separator: &str, depth: u8, line: std::io::Result<String>) -> Result<(u64, f64, f64), Box<dyn Error>> {
          let area_per_cell = (PI / 3.0) / (1_u64 << (depth << 1) as u32) as f64;  // = 4pi / (12*4^depth)
          let line = line?;
          let (uniq, val) = line.trim()
            .split_once(separator)
            .ok_or_else(|| String::from("split on space failed."))?;
          let uniq = uniq.parse::<u64>()?;
          let val  =  val.parse::<f64>()?;
          let (cdepth, _) = Hpx::<u64>::from_uniq_hpx(uniq);
          if cdepth > depth {
            return Err(format!("Cell depth {} larger than MOC depth {} not supported.", cdepth, depth).into());
          }
          let n_sub_cells = (1_u64 << (((depth - cdepth) << 1) as u32)) as f64;
          Ok((uniq, val, val / (n_sub_cells * area_per_cell)))
        }
        fn line2uvd_from_dens(separator: &str, depth: u8, line: std::io::Result<String>) -> Result<(u64, f64, f64), Box<dyn Error>> {
          let area_per_cell = (PI / 3.0) / (1_u64 << (depth << 1) as u32) as f64;  // = 4pi / (12*4^depth)
          let line = line?;
          let (uniq, dens) = line.trim()
            .split_once(separator)
            .ok_or_else(|| String::from("split on space failed."))?;
          let uniq = uniq.parse::<u64>()?;
          let dens = dens.parse::<f64>()?;
          let (cdepth, _ipix) = Hpx::<u64>::from_uniq_hpx(uniq);
          if cdepth > depth {
            return Err(format!("Cell depth {} larger than MOC depth {} not supported.", cdepth, depth).into());
          }
          let n_sub_cells = (1_u64 << (((depth - cdepth) << 1) as u32)) as f64;
          Ok((uniq, dens * n_sub_cells * area_per_cell, dens))
        }
        let ranges: HpxRanges<u64> = if density {
          let line2uniq_val_dens = move |line: std::io::Result<String>| {
            match line2uvd_from_dens(&separator, depth, line) {
              Ok(uniq_val_dens) => Some(uniq_val_dens),
              Err(e) => {
                eprintln!("Error reading or parsing line: {:?}", e);
                None
              }
            }
          };
         valued_cells_to_moc_with_opt::<u64, f64>(
            depth,
            if input == PathBuf::from(r"-") {
              let stdin = std::io::stdin();
              stdin.lock().lines().filter_map(line2uniq_val_dens).collect()
            } else {
              let f = File::open(input)?;
              let reader = BufReader::new(f);
              reader.lines().filter_map(line2uniq_val_dens).collect()
            },
            from_threshold, to_threshold, asc, !not_strict, !split, revese_recursive_descent
          )
        } else {
          let line2uniq_val_dens = move |line: std::io::Result<String>| {
            match line2uvd_from_val(&separator, depth, line) {
              Ok(uniq_val_dens) => Some(uniq_val_dens),
              Err(e) => {
                eprintln!("Error reading or parsing line: {:?}", e);
                None
              }
            }
          };
          valued_cells_to_moc_with_opt::<u64, f64>(
            depth,
            if input == PathBuf::from(r"-") {
              let stdin = std::io::stdin();
              stdin.lock().lines().filter_map(line2uniq_val_dens).collect()
            } else {
              let f = File::open(input)?;
              let reader = BufReader::new(f);
              reader.lines().filter_map(line2uniq_val_dens).collect()
            },
            from_threshold, to_threshold, asc, !not_strict, !split, revese_recursive_descent
          )
        };
        let moc = RangeMOC::new(depth, ranges);
        out.write_smoc_possibly_auto_converting_from_u64(moc.into_range_moc_iter())
      },
      From::Timestamp {
        depth,
        time,
        input,
        out
      } => {
        let line2ts = move |line: std::io::Result<String>| {
          match line.map_err(|e| e.into()).and_then(|s| time.parse(&s)) {
            Ok(t) => Some(t),
            Err(e) => {
              eprintln!("Error reading or parsing line: {:?}", e);
              None
            }
          }
        };
        if input == PathBuf::from(r"-") {
          let stdin = std::io::stdin();
          out.write_tmoc_possibly_auto_converting_from_u64(
            RangeMOC::<u64, Time::<u64>>::from_microsec_since_jd0(
              depth, stdin.lock().lines().filter_map(line2ts), None
            ).into_range_moc_iter()
          )
        } else {
          let f = File::open(input)?;
          let reader = BufReader::new(f);
          out.write_tmoc_possibly_auto_converting_from_u64(
            RangeMOC::<u64, Time::<u64>>::from_microsec_since_jd0(
              depth, reader.lines().filter_map(line2ts), None
            ).into_range_moc_iter()
          )
        }
      },
      From::Timerange {
        depth,
        time,
        input,
        separator,
        out
      } => {
        fn line2tr(separator: &str, time: &InputTime, line: std::io::Result<String>) -> Result<Range<u64>, Box<dyn Error>> {
          let line = line?;
          let (tmin, tmax) = line.trim()
            .split_once(&separator)
            .ok_or_else(|| String::from("split on space failed."))?;
          let tmin = time.parse(tmin)?;
          let tmax = time.parse(tmax)?;
          Ok(tmin..tmax)
        }
        let line2trange = move |line: std::io::Result<String>| {
          match line2tr(&separator, &time, line) {
            Ok(trange) => Some(trange),
            Err(e) => {
              eprintln!("Error reading or parsing line: {:?}", e);
              None
            }
          }
        };
        if input == PathBuf::from(r"-") {
          let stdin = std::io::stdin();
          out.write_tmoc_possibly_auto_converting_from_u64(
            RangeMOC::<u64, Time::<u64>>::from_microsec_ranges_since_jd0(
              depth, stdin.lock().lines().filter_map(line2trange), None
            ).into_range_moc_iter()
          )
        } else {
          let f = File::open(input)?;
          let reader = BufReader::new(f);
          out.write_tmoc_possibly_auto_converting_from_u64(
            RangeMOC::<u64, Time::<u64>>::from_microsec_ranges_since_jd0(
              depth, reader.lines().filter_map(line2trange), None
            ).into_range_moc_iter()
          )
        }
      },
      From::TimestampPos {
        tdepth,
        sdepth,
        time,
        input,
        separator,
        out
      } => {
        let time_shift = Time::<u64>::shift_from_depth_max(tdepth) as u32;
        let layer = healpix::nested::get(sdepth);
        fn line2tscoos(
          separator: &str,
          time: &InputTime, 
          layer: &Layer,
          time_shift: u32,
          line: std::io::Result<String>
        ) -> Result<(u64, u64), Box<dyn Error>> {
          let line = line?;
          let (time_str, line) = line.trim()
            .split_once(separator)
            .ok_or_else(|| String::from("split to separate time from space failed."))?;
          let (lon_deg, lat_deg) = line.split_once(separator)
            .ok_or_else(|| String::from("split on space failed."))?;
          let time_us = time.parse(&time_str)?;
          let lon_deg = lon_deg.parse::<f64>()?;
          let lat_deg = lat_deg.parse::<f64>()?;
          let lon = lon_deg2rad(lon_deg)?;
          let lat = lat_deg2rad(lat_deg)?;
          let time_idx = time_us >> time_shift;
          let hpx = layer.hash(lon, lat);
          Ok((time_idx, hpx))
        }
        let line2tpos = move |line: std::io::Result<String>| {
          match line2tscoos(&separator, &time, &layer, time_shift, line) {
            Ok(lonlat) => Some(lonlat),
            Err(e) => {
              eprintln!("Error reading or parsing line: {:?}", e);
              None
            }
          }
        };
        let moc2: RangeMOC2<u64, Time<u64>, u64, Hpx<u64>> = if input == PathBuf::from(r"-") {
          let stdin = std::io::stdin();
          RangeMOC2::from_fixed_depth_cells(tdepth, sdepth, stdin.lock().lines().filter_map(line2tpos), None)
        } else {
          let f = File::open(input)?;
          let reader = BufReader::new(f);
          RangeMOC2::from_fixed_depth_cells(tdepth, sdepth, reader.lines().filter_map(line2tpos), None)
        };
        out.write_stmoc(moc2.into_range_moc2_iter())
      },
      From::TimerangePos {
        tdepth,
        sdepth,
        time,
        input,
        separator,
        out
      } => {
        let layer = healpix::nested::get(sdepth);
        fn line2trcoos(
          separator: &str,
          time: &InputTime,
          layer: &Layer,
          line: std::io::Result<String>
        ) -> Result<(Range<u64>, u64), Box<dyn Error>> {
          let line = line?;
          let (tmin, line) = line.trim()
            .split_once(separator)
            .ok_or_else(|| String::from("split to isolate tmin failed."))?;
          let (tmax, line) = line.trim()
            .split_once(separator)
            .ok_or_else(|| String::from("split to isolate tmax failed."))?;
          let (lon_deg, lat_deg) = line.split_once(separator)
            .ok_or_else(|| String::from("split on space failed."))?;
          let tmin = time.parse(tmin)?;
          let tmax = time.parse(tmax)?;
          if tmin > tmax {
            return Err(format!("tmin > tmax: {} > {}", tmin, tmax).into());
          }
          let lon_deg = lon_deg.parse::<f64>()?;
          let lat_deg = lat_deg.parse::<f64>()?;
          let lon = lon_deg2rad(lon_deg)?;
          let lat = lat_deg2rad(lat_deg)?;
          let hpx = layer.hash(lon, lat);
          Ok((tmin..tmax, hpx))
        }
        let line2trpos = move |line: std::io::Result<String>| {
          match line2trcoos(&separator, &time, &layer, line) {
            Ok(lonlat) => Some(lonlat),
            Err(e) => {
              eprintln!("Error reading or parsing line: {:?}", e);
              None
            }
          }
        };
        let moc2: RangeMOC2<u64, Time<u64>, u64, Hpx<u64>> = if input == PathBuf::from(r"-") {
          let stdin = std::io::stdin();
          RangeMOC2::from_ranges_and_fixed_depth_cells(tdepth, sdepth, stdin.lock().lines().filter_map(line2trpos), None)
        } else {
          let f = File::open(input)?;
          let reader = BufReader::new(f);
          RangeMOC2::from_ranges_and_fixed_depth_cells(tdepth, sdepth, reader.lines().filter_map(line2trpos), None)
        };
        out.write_stmoc(moc2.into_range_moc2_iter())
      }
      // ST-MOC from t-moc + s-moc (we can then create a complex ST-MOC by union of elementary ST-MOCs)
      // - e.g. multiple observation of the same area of the sky
      // - XMM ST-MOC (from list of observations)?
    }
  }
}

fn lon_deg2rad(lon_deg: f64) -> Result<f64, Box<dyn Error>> {
  let lon = lon_deg.to_radians();
  if lon < 0.0 || TWICE_PI <= lon {
    Err(String::from("Longitude must be in [0, 2pi[").into())
  } else {
    Ok(lon)
  }
}

fn lat_deg2rad(lat_deg: f64) -> Result<f64, Box<dyn Error>> {
  let lat  = lat_deg.to_radians();
  if lat < -HALF_PI || HALF_PI <= lat {
    Err(String::from("Latitude must be in [-pi/2, pi/2]").into())
  } else {
    Ok(lat)
  }
}


#[cfg(test)]
mod tests {

  use std::fs;
  use std::path::PathBuf;
  
  use crate::from::From;
  use crate::output::OutputFormat;
  use crate::InputTime;

  // Yes, I could have mad a single function with different parameters... 

  #[test]
  fn test_from_ring() {
    let expected = "test/resources/test.from_ring.expected.txt";
    let actual = "test/resources/test.from_ring.actual.txt";
    let from = From::Ring {
      depth: 10,
      lon_deg: 13.158329,
      lat_deg: -72.80028,
      r_int_deg: 5.64323,
      r_ext_deg: 10.0,
      out: OutputFormat::Ascii {
        fold: Some(80),
        range_len: false,
        opt_file: Some(actual.into()),
      }
    };
    from.exec().unwrap();
    // Check results
    let actual = fs::read_to_string(actual).unwrap();
    let expected = fs::read_to_string(expected).unwrap();
    assert_eq!(actual, expected);
  }

  #[test]
  fn test_from_valued_cells_1() {
    let expected = "test/resources/gw190425z_skymap.default.expected.txt";
    let actual = "test/resources/gw190425z_skymap.default.actual.txt";
    let from = From::ValuedCells {
      depth: 8,
      density: true,
      from_threshold: String::from("0"),
      to_threshold: String::from("0.9"),
      asc: false,
      not_strict: true,
      split: true,
      revese_recursive_descent: false,
      input: PathBuf::from("test/resources/gw190425z_skymap.multiorder.csv"),
      separator: String::from(","),
      out: OutputFormat::Ascii {
        fold: Some(80),
        range_len: false,
        opt_file: Some(actual.into()),
      }
    };
    from.exec().unwrap();
    // Check results
    let actual = fs::read_to_string(actual).unwrap();
    let expected = fs::read_to_string(expected).unwrap();
    assert_eq!(actual, expected);
  }

  #[test]
  fn test_from_valued_cells_2() {
    let expected = "test/resources/gw190425z_skymap.rrd.expected.txt";
    let actual = "test/resources/gw190425z_skymap.rrd.actual.txt";
    let from = From::ValuedCells {
      depth: 8,
      density: true,
      from_threshold: String::from("0"),
      to_threshold: String::from("0.9"),
      asc: false,
      not_strict: true,
      split: true,
      revese_recursive_descent: true,
      input: PathBuf::from("test/resources/gw190425z_skymap.multiorder.csv"),
      separator: String::from(","),
      out: OutputFormat::Ascii {
        fold: Some(80),
        range_len: false,
        opt_file: Some(actual.into()),
      }
    };
    from.exec().unwrap();
    // Check results
    let actual = fs::read_to_string(actual).unwrap();
    let expected = fs::read_to_string(expected).unwrap();
    assert_eq!(actual, expected);
  }

  #[test]
  fn test_from_valued_cells_3() {
    let expected = "test/resources/gw190425z_skymap.strict.expected.txt";
    let actual = "test/resources/gw190425z_skymap.strict.actual.txt";
    let from = From::ValuedCells {
      depth: 8,
      density: true,
      from_threshold: String::from("0"),
      to_threshold: String::from("0.9"),
      asc: false,
      not_strict: false,
      split: true,
      revese_recursive_descent: false,
      input: PathBuf::from("test/resources/gw190425z_skymap.multiorder.csv"),
      separator: String::from(","),
      out: OutputFormat::Ascii {
        fold: Some(80),
        range_len: false,
        opt_file:  Some(actual.into()),
      }
    };
    from.exec().unwrap();
    // Check results
    let actual = fs::read_to_string(actual).unwrap();
    let expected = fs::read_to_string(expected).unwrap();
    assert_eq!(actual, expected);
  }

  #[test]
  fn test_from_valued_cells_4() {
    let expected = "test/resources/gw190425z_skymap.strict.rrd.expected.txt";
    let actual = "test/resources/gw190425z_skymap.strict.rrd.actual.txt";
    let from = From::ValuedCells {
      depth: 8,
      density: true,
      from_threshold: String::from("0"),
      to_threshold: String::from("0.9"),
      asc: false,
      not_strict: false,
      split: true,
      revese_recursive_descent: true,
      input: PathBuf::from("test/resources/gw190425z_skymap.multiorder.csv"),
      separator: String::from(","),
      out: OutputFormat::Ascii {
        fold: Some(80),
        range_len: false,
        opt_file:  Some(actual.into()),
      }
    };
    from.exec().unwrap();
    // Check results
    let actual = fs::read_to_string(actual).unwrap();
    let expected = fs::read_to_string(expected).unwrap();
    assert_eq!(actual, expected);
  }

  #[test]
  fn test_from_valued_cells_5() {
    let expected = "test/resources/gw190425z_skymap.strict.nosplit.expected.txt";
    let actual = "test/resources/gw190425z_skymap.strict.nosplit.actual.txt";
    let from = From::ValuedCells {
      depth: 8,
      density: true,
      from_threshold: String::from("0"),
      to_threshold: String::from("0.9"),
      asc: false,
      not_strict: false,
      split: false,
      revese_recursive_descent: false,
      input: PathBuf::from("test/resources/gw190425z_skymap.multiorder.csv"),
      separator: String::from(","),
      out: OutputFormat::Ascii {
        fold: Some(80),
        range_len: false,
        opt_file:  Some(actual.into()),
      }
    };
    from.exec().unwrap();
    // Check results
    let actual = fs::read_to_string(actual).unwrap();
    let expected = fs::read_to_string(expected).unwrap();
    assert_eq!(actual, expected);
  }

  #[test]
  fn test_from_valued_cells_6() {
    let expected = "test/resources/gw190425z_skymap.nosplit.expected.txt";
    let actual = "test/resources/gw190425z_skymap.nosplit.actual.txt";
    let from = From::ValuedCells {
      depth: 8,
      density: true,
      from_threshold: String::from("0"),
      to_threshold: String::from("0.9"),
      asc: false,
      not_strict: true,
      split: false,
      revese_recursive_descent: false,
      input: PathBuf::from("test/resources/gw190425z_skymap.multiorder.csv"),
      separator: String::from(","),
      out: OutputFormat::Ascii {
        fold: Some(80),
        range_len: false,
        opt_file:  Some(actual.into()),
      }
    };
    from.exec().unwrap();
    // Check results
    let actual = fs::read_to_string(actual).unwrap();
    let expected = fs::read_to_string(expected).unwrap();
    assert_eq!(actual, expected);
  }
  
  #[test]
  fn st_moc_logxmm_range() {
    // cat xmmlog.psv | tr '|' ',' | sed -r 's/ *, */,/g' | cut -d , -f '1,2,9,10' | awk -F , '{print $3","$4","$1","$2}' > xmmlog.csv
    let from = From::TimerangePos {
      tdepth: 24, //33,
      sdepth: 7, //17,
      time: InputTime::IsoSimple,
      input: PathBuf::from("test/resources/xmmlog.csv"),
      separator: String::from(","),
      out: OutputFormat::Fits {
        force_u64: true,
        moc_id: None,
        moc_type: None,
        file: PathBuf::from("test/resources/xmmlog.range.stmoc.fits")
      }
    };
    from.exec().unwrap();
  }

  
  #[test]
  fn st_moc_logxmm_val_1() {
    // xmmlog1.csv from xmmlog.vot using TOPCAT
    let expected = "test/resources/xmmlog.stmoc.t24.s7.expected.txt";
    let actual = "test/resources/xmmlog.stmoc.t24.s7.actual.txt";
    let from = From::TimestampPos {
      tdepth: 24,
      sdepth: 7,
      time: InputTime::JD,
      input: PathBuf::from("test/resources/xmmlog1.csv"),
      separator: String::from(","),
      out: OutputFormat::Ascii {
        fold: Some(80),
        range_len: false,
        opt_file: Some(actual.into()),
      }
      /*out: OutputFormat::Fits {
        force_u64: true,
        moc_id: None,
        moc_type: None,
        file: PathBuf::from("test/resources/xmmlog.val.stmoc.fits")
      }*/
    };
    from.exec().unwrap();
    // Check results
    let actual = fs::read_to_string(actual).unwrap();
    let expected = fs::read_to_string(expected).unwrap();
    assert_eq!(actual, expected);
  }

  #[test]
  fn st_moc_logxmm_val_2() {
    // xmmlog1.csv from xmmlog.vot using TOPCAT
    let expected = "test/resources/xmmlog.stmoc.t35.s10.expected.txt";
    let actual = "test/resources/xmmlog.stmoc.t35.s10.actual.txt";
    let from = From::TimestampPos {
      tdepth: 35,
      sdepth: 10,
      time: InputTime::JD,
      input: PathBuf::from("test/resources/xmmlog1.csv"),
      separator: String::from(","),
      out: OutputFormat::Ascii {
        fold: Some(80),
        range_len: false,
        opt_file: Some(actual.into()),
      }
      /*out: OutputFormat::Ascii {
        force_u64: true,
        moc_id: None,
        moc_type: None,
        file: PathBuf::from(actual.into())
      }*/
    };
    from.exec().unwrap();
    // Check results
    let actual = fs::read_to_string(actual).unwrap();
    let expected = fs::read_to_string(expected).unwrap();
    assert_eq!(actual, expected);
  }
  
}
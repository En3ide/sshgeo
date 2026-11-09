//! analyse les échecs SSH et sort les statistiques par IP et par pays sans accès réseau

use std::{
    collections::{BTreeMap, HashMap},
    env,
    fs::{self, File},
    io::{self, BufRead, BufReader, BufWriter, Write},
    net::IpAddr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

// les bases sont dans le binaire, donc rien à télécharger au lancement
const V4: &[u8] = include_bytes!("../data/geoip-v4.bin");
const V6: &[u8] = include_bytes!("../data/geoip-v6.bin");
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
/// garde les options passées à la commande
struct Args {
    logs: Vec<String>,
    journal: bool,
    since: Option<i64>,
    until: Option<i64>,
    output: PathBuf,
    syslog_year: Option<i32>,
    syslog_offset: i64,
}

#[derive(Clone, Debug)]
/// garde le nombre d'échecs et les dates de la première et de la dernière tentative
struct Stat {
    count: u64,
    first: i64,
    last: i64,
}
impl Stat {
    /// ajoute des échecs au compteur et met à jour les dates si besoin
    fn add(&mut self, count: u64, first: i64, last: i64) {
        self.count += count;
        self.first = self.first.min(first);
        self.last = self.last.max(last);
    }
}

/// transforme une date en nombre de jours depuis le 1er janvier 1970
fn days(year: i32, month: u32, day: u32) -> i64 {
    let y = year as i64 - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yo = y - era * 400;
    let m = month as i64 + if month > 2 { -3 } else { 9 };
    era * 146097 + yo * 365 + yo / 4 - yo / 100 + (153 * m + 2) / 5 + day as i64 - 1 - 719468
}

/// retrouve l'année, le mois et le jour à partir du nombre de jours depuis 1970
fn civil(day: i64) -> (i32, u32, u32) {
    let z = day + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yo = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yo + era * 400;
    let doy = doe - (yo * 365 + yo / 4 - yo / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    ((y + i64::from(m <= 2)) as i32, m as u32, d as u32)
}

/// vérifie que la date existe puis la convertit en secondes depuis 1970
fn date_seconds(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> Option<i64> {
    if !(1..=9999).contains(&y) || !(1..=12).contains(&m) || d == 0 || h > 23 || min > 59 || s > 59
    {
        return None;
    }
    let day = days(y, m, d);
    if civil(day) != (y, m, d) {
        return None;
    }
    Some(day * 86400 + h as i64 * 3600 + min as i64 * 60 + s as i64)
}

/// récupère un nombre dans une partie du texte sans planter si le format est mauvais
fn number(s: &str, a: usize, b: usize) -> Option<u32> {
    s.get(a..b)?.parse().ok()
}
/// lit le décalage du fuseau horaire et le convertit en secondes
fn offset(s: &str) -> Option<i64> {
    if s == "Z" || s.is_empty() {
        return Some(0);
    }
    let sign = match s.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (h, m) = if s.len() == 6 && s.get(3..4)? == ":" {
        (number(s, 1, 3)?, number(s, 4, 6)?)
    } else if s.len() == 5 {
        (number(s, 1, 3)?, number(s, 3, 5)?)
    } else {
        return None;
    };
    if h > 23 || m > 59 {
        return None;
    }
    Some(sign * (h as i64 * 3600 + m as i64 * 60))
}

/// lit une date ISO et la convertit en microsecondes UTC, avec UTC par défaut si le fuseau manque
fn iso(s: &str) -> Option<i64> {
    if s.get(4..5)? != "-" || s.get(7..8)? != "-" {
        return None;
    }
    let (y, m, d) = (number(s, 0, 4)? as i32, number(s, 5, 7)?, number(s, 8, 10)?);
    if s.len() == 10 {
        return Some(date_seconds(y, m, d, 0, 0, 0)? * 1_000_000);
    }
    if !matches!(s.get(10..11)?, "T" | "t") || s.get(13..14)? != ":" || s.get(16..17)? != ":" {
        return None;
    }
    let seconds = date_seconds(
        y,
        m,
        d,
        number(s, 11, 13)?,
        number(s, 14, 16)?,
        number(s, 17, 19)?,
    )?;
    let mut rest = s.get(19..)?;
    let mut micros = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let length = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if length == 0 {
            return None;
        }
        for i in 0..6 {
            micros = micros * 10
                + fraction
                    .as_bytes()
                    .get(i)
                    .filter(|_| i < length)
                    .map_or(0, |b| (b - b'0') as i64);
        }
        rest = fraction.get(length..)?;
    }
    Some((seconds - offset(rest)?) * 1_000_000 + micros)
}

/// remet la date en texte au format ISO avec le fuseau UTC pour les CSV
fn formatted(stamp: i64) -> String {
    let seconds = stamp.div_euclid(1_000_000);
    let (y, m, d) = civil(seconds.div_euclid(86400));
    let sec = seconds.rem_euclid(86400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:06}+00:00",
        sec / 3600,
        sec / 60 % 60,
        sec % 60,
        stamp.rem_euclid(1_000_000)
    )
}

/// récupère la date d'une ligne de log, même avec l'ancien format syslog sans année
fn timestamp(line: &str, reference: i64, args: &Args) -> Option<i64> {
    let mut fields = line.split_whitespace();
    let first = fields.next()?;
    if first.as_bytes().get(4) == Some(&b'-') {
        return iso(first);
    }
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|&m| m == first)? as u32
        + 1;
    let day = fields.next()?.parse().ok()?;
    let time = fields.next()?;
    let (h, min, s) = (
        number(time, 0, 2)?,
        number(time, 3, 5)?,
        number(time, 6, 8)?,
    );
    let mut year = args
        .syslog_year
        .unwrap_or(civil(reference.div_euclid(86400)).0);
    let mut seconds = date_seconds(year, month, day, h, min, s)? - args.syslog_offset;
    // un log de décembre archivé en janvier appartient à l'année précédente
    if args.syslog_year.is_none() && seconds > reference + 86400 {
        year -= 1;
        seconds = date_seconds(year, month, day, h, min, s)? - args.syslog_offset;
    }
    Some(seconds * 1_000_000)
}

/// sort l'IP d'une ligne qui indique un échec de mot de passe SSH
fn failed_ip(line: &str) -> Option<&str> {
    let (prefix, message) = line.split_once("Failed password for ")?;
    if !prefix.contains("sshd") {
        return None;
    }
    // on part de la fin car le nom d'utilisateur peut lui aussi contenir ce texte
    let (_, tail) = message.rsplit_once(" from ")?;
    let mut fields = tail.split_whitespace();
    let ip = fields.next()?;
    if fields.next()? != "port" || fields.next()?.parse::<u16>().is_err() {
        return None;
    }
    Some(ip)
}

/// lit les logs ligne par ligne et compte les échecs par IP sur la période demandée
fn consume(
    reader: impl BufRead,
    reference: i64,
    args: &Args,
    stats: &mut HashMap<IpAddr, Stat>,
) -> Result<()> {
    let mut reader = reader;
    let mut buf = Vec::with_capacity(1024);
    while reader.read_until(b'\n', &mut buf)? != 0 {
        let line = String::from_utf8_lossy(&buf);
        if let Some(raw_ip) = failed_ip(&line) {
            let stamp = timestamp(&line, reference, args)
                .ok_or_else(|| format!("Horodatage illisible : {}", line.trim_end()))?;
            if args.since.map_or(true, |s| stamp >= s) && args.until.map_or(true, |s| stamp < s) {
                let ip: IpAddr = raw_ip
                    .parse()
                    .map_err(|_| format!("IP invalide : {raw_ip}"))?;
                // une IPv4 écrite sous forme IPv6 doit rester dans le même compteur
                let ip = match ip {
                    IpAddr::V6(v) => v.to_ipv4_mapped().map_or(ip, IpAddr::V4),
                    _ => ip,
                };
                stats
                    .entry(ip)
                    .and_modify(|s| s.add(1, stamp, stamp))
                    .or_insert(Stat {
                        count: 1,
                        first: stamp,
                        last: stamp,
                    });
            }
        }
        buf.clear();
    }
    Ok(())
}

/// cherche le pays dans la base intégrée et renvoie ?? si l'IP n'est pas couverte
fn country(ip: IpAddr) -> [u8; 2] {
    let (data, width, value) = match ip {
        IpAddr::V4(v) => (V4, 4, u32::from(v) as u128),
        IpAddr::V6(v) => (V6, 16, u128::from(v)),
    };
    // chaque entrée contient le début et la fin de la plage, puis les deux lettres du pays
    let stride = width * 2 + 2;
    let read = |index: usize| -> u128 {
        data[index..index + width]
            .iter()
            .fold(0, |v, &b| (v << 8) | b as u128)
    };
    // la recherche binaire évite de parcourir toute la base pour chaque IP
    let mut lo = 0;
    let mut hi = data.len() / stride;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if read(mid * stride) <= value {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        return *b"??";
    }
    let pos = (lo - 1) * stride;
    if value > read(pos + width) {
        return *b"??";
    }
    [data[pos + width * 2], data[pos + width * 2 + 1]]
}

/// traduit les noms de pays courants et garde le code ISO pour les autres
fn country_name(cc: &str) -> &str {
    match cc {
        "NL" => "Pays-Bas",
        "FR" => "France",
        "AD" => "Andorre",
        "CN" => "Chine",
        "SG" => "Singapour",
        "GB" => "Royaume-Uni",
        "US" => "États-Unis",
        "CH" => "Suisse",
        "IL" => "Israël",
        "KZ" => "Kazakhstan",
        "VN" => "Vietnam",
        "KR" => "Corée du Sud",
        "RO" => "Roumanie",
        "DE" => "Allemagne",
        "RU" => "Russie",
        "UA" => "Ukraine",
        "IN" => "Inde",
        "BR" => "Brésil",
        "CA" => "Canada",
        "JP" => "Japon",
        "HK" => "Hong Kong",
        "TW" => "Taïwan",
        "ID" => "Indonésie",
        "TR" => "Turquie",
        "IT" => "Italie",
        "ES" => "Espagne",
        "PL" => "Pologne",
        "BE" => "Belgique",
        "AU" => "Australie",
        "TH" => "Thaïlande",
        "IR" => "Iran",
        "PK" => "Pakistan",
        "BD" => "Bangladesh",
        "SE" => "Suède",
        "NO" => "Norvège",
        "FI" => "Finlande",
        "DK" => "Danemark",
        "CZ" => "Tchéquie",
        "BG" => "Bulgarie",
        "LT" => "Lituanie",
        "LV" => "Lettonie",
        "EE" => "Estonie",
        "IE" => "Irlande",
        "PT" => "Portugal",
        "MX" => "Mexique",
        "AR" => "Argentine",
        "ZA" => "Afrique du Sud",
        "AE" => "Émirats arabes unis",
        "SA" => "Arabie saoudite",
        "??" => "Inconnu",
        _ => cc,
    }
}

/// écrit une ligne CSV et protège les champs qui contiennent des caractères spéciaux
fn csv_row(out: &mut impl Write, cells: &[String]) -> io::Result<()> {
    for (i, cell) in cells.iter().enumerate() {
        if i != 0 {
            write!(out, ";")?;
        }
        if cell.contains([';', '"', '\n', '\r']) {
            write!(out, "\"{}\"", cell.replace('"', "\"\""))?;
        } else {
            write!(out, "{cell}")?;
        }
    }
    writeln!(out)
}

/// crée les deux CSV, trie les résultats et affiche les pays les plus présents
fn export(stats: &HashMap<IpAddr, Stat>, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    let mut detail = BufWriter::new(File::create(dir.join("detail_ip.csv"))?);
    write!(
        detail,
        "\u{feff}IP;Pays_ISO;Pays;Echecs_SSH;Premiere_tentative_UTC;Derniere_tentative_UTC\n"
    )?;
    let mut by_ip: Vec<_> = stats.iter().collect();
    by_ip.sort_by(|(a, x), (b, y)| y.count.cmp(&x.count).then_with(|| a.cmp(b)));
    let mut by_country: BTreeMap<[u8; 2], (Stat, u64)> = BTreeMap::new();
    for (&ip, stat) in by_ip {
        let cc = country(ip);
        let name = std::str::from_utf8(&cc)?;
        csv_row(
            &mut detail,
            &[
                ip.to_string(),
                name.into(),
                country_name(name).into(),
                stat.count.to_string(),
                formatted(stat.first),
                formatted(stat.last),
            ],
        )?;
        by_country
            .entry(cc)
            .and_modify(|(s, ips)| {
                s.add(stat.count, stat.first, stat.last);
                *ips += 1;
            })
            .or_insert((stat.clone(), 1));
    }
    detail.flush()?;
    let total: u64 = stats.values().map(|s| s.count).sum();
    let mut ordered: Vec<_> = by_country.iter().collect();
    ordered.sort_by(|(a, (x, _)), (b, (y, _))| y.count.cmp(&x.count).then_with(|| a.cmp(b)));
    let mut countries = BufWriter::new(File::create(dir.join("classement_pays.csv"))?);
    write!(countries, "\u{feff}Pays_ISO;Pays;Echecs_SSH;IP_distinctes;Part_pourcent;Premiere_tentative_UTC;Derniere_tentative_UTC\n")?;
    println!(
        "{total} échecs SSH | {} IP | {} pays",
        stats.len(),
        by_country.len()
    );
    for (i, (cc, (s, ips))) in ordered.iter().enumerate() {
        let name = std::str::from_utf8(*cc)?;
        let percent = s.count as f64 * 100.0 / total as f64;
        csv_row(
            &mut countries,
            &[
                name.into(),
                country_name(name).into(),
                s.count.to_string(),
                ips.to_string(),
                format!("{percent:.2}"),
                formatted(s.first),
                formatted(s.last),
            ],
        )?;
        if i < 10 {
            println!(
                "  {name} {:22} {:>9} échecs {:>5} IP {percent:>6.2} %",
                country_name(name),
                s.count,
                ips
            );
        }
    }
    countries.flush()?;
    println!("CSV : {}", fs::canonicalize(dir)?.display());
    Ok(())
}

/// vérifie si un nom de fichier correspond au motif, avec * comme joker
fn matches(pattern: &str, name: &str) -> bool {
    let parts: Vec<_> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    if !name.starts_with(parts[0]) {
        return false;
    }
    let mut rest = &name[parts[0].len()..];
    for part in &parts[1..parts.len() - 1] {
        if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
    }
    rest.ends_with(parts[parts.len() - 1])
}

/// retrouve les fichiers demandés, que le chemin soit exact ou contienne un joker
fn expand(pattern: &str) -> Result<Vec<PathBuf>> {
    if !pattern.contains('*') {
        let path = PathBuf::from(pattern);
        if !path.is_file() {
            return Err(format!("Fichier absent : {pattern}").into());
        }
        return Ok(vec![path]);
    }
    let path = Path::new(pattern);
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or("Motif invalide")?
        .to_str()
        .ok_or("Chemin non UTF-8")?;
    let mut paths = Vec::new();
    if !dir.exists() {
        return Ok(paths);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && matches(name, &entry.file_name().to_string_lossy()) {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}

/// lit la sortie de gzip ou journalctl et remonte une erreur si la commande échoue
fn read_child(
    mut command: Command,
    args: &Args,
    reference: i64,
    stats: &mut HashMap<IpAddr, Stat>,
) -> Result<()> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let result = consume(
        BufReader::with_capacity(262144, child.stdout.take().ok_or("Sortie absente")?),
        reference,
        args,
        stats,
    );
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    result?;
    if !status.success() {
        return Err(format!("Échec de lecture du journal : {status}").into());
    }
    Ok(())
}

/// choisit les journaux à lire, lance le comptage puis génère les CSV
fn run(args: Args) -> Result<()> {
    let started = Instant::now();
    let mut stats = HashMap::new();
    let mut files = Vec::new();
    if !args.journal {
        if args.logs.is_empty() {
            files.extend(expand("/var/log/auth.log*")?);
            files.extend(expand("/var/log/secure*")?);
        } else {
            for pattern in &args.logs {
                let paths = expand(pattern)?;
                if paths.is_empty() {
                    return Err(format!("Aucun fichier : {pattern}").into());
                }
                files.extend(paths);
            }
        }
    }
    if !files.is_empty() {
        // on ne lit qu'une fois un fichier sélectionné par plusieurs motifs
        let mut canonical = Vec::new();
        for path in files {
            canonical.push(fs::canonicalize(path)?);
        }
        canonical.sort();
        canonical.dedup();
        for path in canonical {
            eprintln!("Lecture : {}", path.display());
            let reference = fs::metadata(&path)?
                .modified()?
                .duration_since(UNIX_EPOCH)?
                .as_secs() as i64;
            if path.extension().map_or(false, |s| s == "gz") {
                let mut command = Command::new("gzip");
                command.arg("-cd").arg("--").arg(&path);
                read_child(command, &args, reference, &mut stats)?;
            } else {
                consume(
                    BufReader::with_capacity(262144, File::open(path)?),
                    reference,
                    &args,
                    &mut stats,
                )?;
            }
        }
    } else {
        // journald sert de source si aucun fichier n'est disponible, ou si on le demande
        eprintln!("Lecture : journald");
        let mut command = Command::new("journalctl");
        command.args([
            "--no-pager",
            "-o",
            "short-iso-precise",
            "--utc",
            "-t",
            "sshd",
            "-t",
            "sshd-session",
            "-t",
            "sshd-auth",
        ]);
        if let Some(s) = args.since {
            command
                .arg("--since")
                .arg(format!("@{}", s.div_euclid(1_000_000)));
        }
        if let Some(s) = args.until {
            command
                .arg("--until")
                .arg(format!("@{}", s.div_euclid(1_000_000) + 1));
        }
        read_child(
            command,
            &args,
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64,
            &mut stats,
        )?;
    }
    export(&stats, &args.output)?;
    println!(
        "Durée : {:.3} s — géolocalisation embarquée, sans réseau",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// affiche les options disponibles et rappelle que la géolocalisation reste hors ligne
fn help() {
    println!(
        "sshgeo 1.0.0 — Rust, entièrement hors ligne\n\
Usage : sshgeo [OPTIONS]\n\
  --since DATE          Début inclus, ISO 8601 ; UTC si fuseau absent\n\
  --until DATE          Fin exclue, ISO 8601\n\
  --log FICHIER         Répétable ; motifs * et fichiers .gz acceptés\n\
  --source journal     Forcer journald (auto par défaut)\n\
  --output-dir DOSSIER  Sortie (défaut : resultats-ssh)\n\
  --syslog-year ANNEE   Année des anciens logs sans année\n\
  --syslog-offset +HH:MM Fuseau des anciens logs (défaut : +00:00)\n\
  --offline            Accepté pour compatibilité ; toujours hors ligne\n\
  --version            Version et date de la base\n\
  -h, --help           Cette aide\n\n\
Base pays : DB-IP, CC BY 4.0, via sapics/ip-location-db.\n\
https://github.com/sapics/ip-location-db/tree/main/dbip-country\n\
Les pays situent les machines sources et non nécessairement les attaquants."
    );
}

/// récupère les options et vérifie les dates ainsi que les combinaisons incompatibles
fn parse() -> Result<Option<Args>> {
    let mut args = Args {
        output: PathBuf::from("resultats-ssh"),
        ..Args::default()
    };
    let mut values = env::args().skip(1);
    while let Some(flag) = values.next() {
        match flag.as_str() {
            "--help" | "-h" => {
                help();
                return Ok(None);
            }
            "--version" | "-V" => {
                println!("sshgeo 1.0.0 — Rust — base DB-IP préparée le 2026-11-09 — hors ligne");
                return Ok(None);
            }
            "--offline" => (),
            "--since" | "--until" => {
                let value = values.next().ok_or("Date manquante")?;
                let stamp = iso(&value).ok_or_else(|| format!("Date ISO invalide : {value}"))?;
                if flag == "--since" {
                    args.since = Some(stamp);
                } else {
                    args.until = Some(stamp);
                }
            }
            "--log" => args.logs.push(values.next().ok_or("Fichier manquant")?),
            "--output-dir" => args.output = PathBuf::from(values.next().ok_or("Dossier manquant")?),
            "--syslog-year" => {
                let year: i32 = values.next().ok_or("Année manquante")?.parse()?;
                if !(1..=9999).contains(&year) {
                    return Err("Année invalide".into());
                }
                args.syslog_year = Some(year);
            }
            "--syslog-offset" => {
                args.syslog_offset =
                    offset(&values.next().ok_or("Fuseau manquant")?).ok_or("Fuseau invalide")?
            }
            "--source" => match values.next().ok_or("Source manquante")?.as_str() {
                "journal" => args.journal = true,
                "auto" => args.journal = false,
                _ => return Err("Source attendue : auto ou journal".into()),
            },
            _ => return Err(format!("Option inconnue : {flag} (voir --help)").into()),
        }
    }
    if args.journal && !args.logs.is_empty() {
        return Err("--log et --source journal sont incompatibles".into());
    }
    if matches!((args.since, args.until), (Some(s), Some(u)) if s >= u) {
        return Err("--until doit être après --since".into());
    }
    Ok(Some(args))
}

/// lance la commande et renvoie un code d'erreur si quelque chose se passe mal
fn main() {
    let result = parse().and_then(|args| {
        if let Some(args) = args {
            run(args)
        } else {
            Ok(())
        }
    });
    if let Err(error) = result {
        eprintln!("Erreur : {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    /// vérifie les fuseaux, les fractions de seconde et le rejet des dates impossibles
    fn dates_and_timezones() {
        assert_eq!(
            iso("2026-11-09T01:42:00+01:00"),
            iso("2026-11-09T00:42:00Z")
        );
        assert_eq!(
            iso("2026-11-09T00:42:00.123456Z").unwrap() % 1_000_000,
            123456
        );
        assert_eq!(iso("2026-11-09T00:42:00.1Z").unwrap() % 1_000_000, 100000);
        assert!(iso("2026-02-29").is_none());
        assert!(iso("2026-13-01").is_none());
        assert!(iso("2026-11-09T24:00:00Z").is_none());
        for s in [
            "1970-01-01T00:00:00Z",
            "2024-02-29T23:59:59.999999Z",
            "2026-12-31T23:59:59Z",
        ] {
            let stamp = iso(s).unwrap();
            assert_eq!(iso(&formatted(stamp)), Some(stamp));
        }
    }
    #[test]
    /// vérifie le comptage, les bornes de dates et l'exclusion des connexions réussies
    fn counting_and_bounds() {
        let text = concat!(
            "2026-11-09T00:00:00Z host sshd[1]: Failed password for root from 1.2.3.4 port 22 ssh2\n",
            "2026-11-09T00:01:00Z host sshd-session[2]: Failed password for invalid user xx from ::ffff:1.2.3.4 port 22 ssh2\n",
            "2026-11-09T00:01:00Z host sshd[3]: Accepted password for ubuntu from 1.2.3.4 port 22 ssh2\n",
            "2026-11-09T00:01:00Z host sshd[4]: Invalid user xx from 1.2.3.4 port 22\n",
            "2026-11-09T00:02:00Z host sshd[5]: Failed password for root from 2001:4860::1 port 22 ssh2\n");
        let args = Args {
            since: iso("2026-11-09"),
            until: iso("2026-11-09T00:02:00Z"),
            ..Args::default()
        };
        let mut stats = HashMap::new();
        consume(io::Cursor::new(text), 0, &args, &mut stats).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats.values().next().unwrap().count, 2);
    }
    #[test]
    /// vérifie les pays de la base embarquée et les motifs de noms de fichiers
    fn geo_and_wildcards() {
        assert_eq!(country("109.160.32.39".parse().unwrap()), *b"NL");
        // cette valeur vient du CSV DB-IP utilisé pour préparer la base embarquée
        assert_eq!(country("2001:4860:4860::8888".parse().unwrap()), *b"CA");
        assert_eq!(country("127.0.0.1".parse().unwrap()), *b"??");
        assert!(matches("auth.log*", "auth.log.1.gz"));
        assert!(!matches("auth.log", "auth.log.1"));
        assert!(matches("auth*log*gz", "auth.log.2.gz"));
    }
    #[test]
    /// vérifie qu'un log de décembre reste dans la bonne année quand on le lit en janvier
    fn syslog_rollover() {
        let reference = iso("2026-01-01").unwrap() / 1_000_000;
        let args = Args::default();
        assert_eq!(
            timestamp("Dec 31 23:59:59 host sshd[1]: text", reference, &args),
            iso("2025-12-31T23:59:59Z")
        );
    }
}

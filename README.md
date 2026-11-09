# sshgeo

Un outil en Rust pour analyser les échecs de connexion SSH et voir de quels pays viennent les IP qui tentent de se connecter à ton serveur

Tu le lances, il lit les journaux et génère deux CSV : un classement par pays et un détail IP par IP

**Tout fonctionne hors ligne**, dès le premier lancement : les bases IPv4 et IPv6 sont intégrées au binaire, sans API ni téléchargement

## Fonctionnalités

- Comptage des échecs de mot de passe SSH par IP et par pays
- Nombre d'IP distinctes et part des tentatives pour chaque pays
- Date de la première et de la dernière tentative
- Lecture des journaux texte, des archives `.gz` et de journald
- Filtrage par date et choix du dossier de sortie
- Affichage des dix premiers pays dans le terminal

Le comptage repose sur les lignes SSH `Failed password`, sans ajouter les connexions réussies ni les événements `Invalid user` seuls

## Installation

Le projet cible Linux et utilise uniquement la bibliothèque standard Rust, sans dépendance Cargo externe

Pour compiler, il faut Rust, Cargo et un compilateur système déjà installés

```bash
git clone https://github.com/En3ide/sshgeo.git
cd sshgeo
cargo build --release --locked --offline
```

La récupération du dépôt nécessite un accès à GitHub, mais la compilation se fait ensuite hors ligne avec les sources et les bases fournies

Pour lancer directement le programme :

```bash
sudo ./target/release/sshgeo
```

Pour installer la commande `sshgeo` :

```bash
sudo install -d -m 755 /usr/local/lib/sshgeo
sudo install -m 755 target/release/sshgeo /usr/local/lib/sshgeo/sshgeo
sudo install -m 755 sshgeo-launcher.sh /usr/local/bin/sshgeo
```

Tu peux ensuite la lancer depuis n'importe quel dossier :

```bash
sshgeo
```

Le lanceur utilise `sudo` pour accéder aux journaux protégés

## Utilisation

Sans option, sshgeo analyse les journaux disponibles et écrit les CSV dans `resultats-ssh/`, à partir du dossier où tu lances la commande

```bash
# analyser les tentatives depuis une date
sshgeo --since 2026-11-01

# choisir une période avec un fuseau explicite
sshgeo --since 2026-11-08T00:00:00+01:00 --until 2026-11-09T00:00:00+01:00

# choisir le dossier des résultats
sshgeo --output-dir /var/lib/sshgeo/resultats

# lire des journaux précis, y compris les archives compressées
sshgeo --log '/var/log/auth.log*'

# utiliser journald
sshgeo --source journal
```

### Options

| Option | Description |
|---|---|
| `--since DATE` | Début de la période, inclus |
| `--until DATE` | Fin de la période, exclue |
| `--log FICHIER` | Fichier ou motif à lire, option répétable |
| `--source auto\|journal` | Détection automatique ou lecture de journald |
| `--output-dir DOSSIER` | Dossier des CSV, `resultats-ssh/` par défaut |
| `--syslog-year ANNEE` | Année à utiliser pour les anciens logs sans année |
| `--syslog-offset +HH:MM` | Décalage horaire des anciens logs, UTC par défaut |
| `--offline` | Accepté pour compatibilité, le programme reste toujours hors ligne |
| `--version` | Version du programme et date de préparation de la base |
| `-h`, `--help` | Aide de la commande |

Les dates utilisent le format ISO 8601, par exemple `2026-11-01` ou `2026-11-01T12:00:00+01:00` ; sans fuseau, elles sont interprétées en UTC

## Les CSV générés

| Fichier | Colonnes |
|---|---|
| `classement_pays.csv` | Code pays, nom, nombre d'échecs, IP distinctes, pourcentage, première et dernière tentative |
| `detail_ip.csv` | IP, code pays, nom, nombre d'échecs, première et dernière tentative |

Les lignes sont triées par nombre d'échecs décroissant et les dates sont exportées en UTC avec leur fuseau explicite

Les fichiers utilisent UTF-8 avec BOM et le séparateur `;` pour faciliter leur ouverture dans Excel

Chaque lancement remplace les deux CSV du dossier choisi ; utilise un autre `--output-dir` si tu veux conserver plusieurs relevés

## Les journaux analysés

Par défaut, sshgeo cherche `/var/log/auth.log*` et `/var/log/secure*`, y compris les fichiers `.gz` ; s'il n'en trouve pas, il passe par journald

Les deux sources ne sont jamais additionnées et un même fichier sélectionné par plusieurs motifs n'est lu qu'une fois

L'option `--log` peut être répétée et accepte le joker `*` dans le nom du fichier ; garde les guillemets autour des motifs pour laisser sshgeo les traiter

La lecture des archives `.gz` utilise la commande système `gzip`, et celle de journald utilise `journalctl` ; ces deux commandes fonctionnent hors ligne et ne sont pas nécessaires pour analyser un fichier texte

Pour une ancienne archive syslog sans année ni fuseau :

```bash
sshgeo --log archive.log --syslog-year 2026 --syslog-offset +01:00
```

Sans ces options, l'année est déduite de la date de modification du fichier et le fuseau utilisé est UTC ; si une archive couvre un changement d'heure, traite les périodes séparément avec leur décalage respectif

## Fonctionnement hors ligne

Les bases des pays sont incluses dans l'exécutable au moment de la compilation avec `include_bytes!`

Une fois compilé, le programme n'a besoin ni du dossier `data/`, ni d'un cache, ni de Python, ni d'un accès réseau ; le binaire utilise toutefois les bibliothèques système Linux et doit être exécuté sur une machine compatible

Les journaux sont lus ligne par ligne pour limiter la mémoire utilisée, et le pays de chaque IP distincte est retrouvé par recherche binaire dans les plages intégrées

## Base des pays

Les données proviennent de [DB-IP](https://db-ip.com/), via [ip-location-db](https://github.com/sapics/ip-location-db/tree/main/dbip-country), sous licence **CC BY 4.0**

La base fournie a été préparée le **9 novembre 2026** ; les empreintes SHA-256 des fichiers sources et des bases binaires sont dans [`data/metadata.json`](data/metadata.json)

Les pays sont identifiés par leur code ISO à deux lettres, avec les noms français courants traduits et `??` pour les IP non couvertes

La base reste figée dans le binaire et ne se met pas à jour automatiquement ; pour préparer une nouvelle version à partir de CSV récupérés séparément :

```bash
python3 prepare_database.py dbip-country-ipv4.csv dbip-country-ipv6.csv
cargo build --release --locked --offline
```

Python sert uniquement à cette étape facultative de préparation, qui travaille sur des fichiers locaux ; il n'est pas nécessaire pour compiler les sources déjà fournies ou lancer sshgeo

Si tu remplaces les bases, actualise aussi leur date dans `--version` et dans ce README

L'attribution complète des données est disponible dans [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)

## Ce que les résultats permettent de savoir

Le pays correspond à la machine source, pas forcément à la personne derrière les tentatives : un VPS, un VPN ou une machine compromise peut servir de relais

Les totaux couvrent uniquement les échecs de mot de passe présents dans les journaux encore disponibles et dans la période choisie ; ils ne constituent pas un historique des DDoS ou des scans de ports

Si une lecture échoue ou si une date est illisible, le programme s'arrête avec un code d'erreur ; un CSV conservé d'un précédent lancement ne doit alors pas être pris pour un nouveau résultat

## Tests et performances

```bash
cargo test --locked --offline
```

Les tests couvrent les dates, les fuseaux, les bornes temporelles, le comptage, les IPv4 mappées en IPv6, la géolocalisation et le passage d'année des logs syslog

Sur une même archive compressée contenant **51 026 échecs et 409 IP**, Rust a pris environ **0,11 seconde**, contre **2,76 secondes** pour la version Python, avec des bases locales et les mêmes résultats ; ces durées dépendent de la machine et du volume des journaux

La compilation et l'exécution ont également été vérifiées dans un espace réseau isolé avec `unshare --net`

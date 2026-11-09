# sshgeo

petit outil en Rust pour voir d'où viennent les tentatives de connexion SSH qui échouent

le but, c'est de récupérer les IP dans les logs, de retrouver leur pays et de sortir deux CSV faciles à lire

ça tourne entièrement hors ligne, même au premier lancement, avec les bases IPv4 et IPv6 directement dans le binaire

## ce que ça fait

- lit les échecs de mot de passe SSH dans les journaux
- compte les tentatives par IP et par pays
- garde la première et la dernière tentative
- génère un classement par pays et un détail IP par IP
- affiche les dix pays les plus présents dans le terminal

les connexions réussies ne sont pas comptées et les lignes `Invalid user` seules ne sont pas ajoutées aux échecs

## compiler

il faut Rust, Cargo et un compilateur système installés sur la machine

```bash
cargo build --release --offline
```

le binaire est dans `target/release/sshgeo`

aucune dépendance Cargo à télécharger et pas besoin de Python pour compiler ou utiliser le programme

si tu préfères compiler directement avec Rust :

```bash
rustc --edition=2021 -O src/main.rs -o sshgeo
```

le projet cible Linux pour la lecture automatique des journaux SSH

## lancer

```bash
sudo ./target/release/sshgeo
```

les deux CSV sont créés dans `resultats-ssh/`, à partir du dossier où tu lances la commande

| fichier | contenu |
|---|---|
| `classement_pays.csv` | pays, nombre d'échecs, IP distinctes, pourcentage et dates |
| `detail_ip.csv` | IP, pays, nombre d'échecs et dates |

les résultats sont triés par nombre d'échecs décroissant et les dates sont en UTC

les CSV utilisent UTF-8 avec BOM et le séparateur `;` pour pouvoir les ouvrir dans Excel

chaque lancement remplace les deux CSV du dossier choisi, donc utilise un autre `--output-dir` si tu veux garder un historique

## en faire une commande

```bash
sudo install -d -m 755 /usr/local/lib/sshgeo
sudo install -m 755 target/release/sshgeo /usr/local/lib/sshgeo/sshgeo
sudo install -m 755 sshgeo-launcher.sh /usr/local/bin/sshgeo
sshgeo
```

le lanceur utilise `sudo` quand il faut les droits administrateur pour lire les journaux

## quelques exemples

```bash
# compter les échecs depuis une date
sshgeo --since 2026-11-01

# choisir une période avec les heures de Paris en hiver
sshgeo --since 2026-11-08T00:00:00+01:00 --until 2026-11-09T00:00:00+01:00

# changer le dossier des CSV
sshgeo --output-dir /var/lib/sshgeo/resultats

# lire des fichiers précis, y compris les archives compressées
sshgeo --log '/var/log/auth.log*'

# passer par journald
sshgeo --source journal

# afficher toutes les options
sshgeo --help
```

`--since` inclut la date de début et `--until` exclut la date de fin

si tu ne précises pas de fuseau, la date est interprétée en UTC

## les journaux

par défaut, le programme cherche `/var/log/auth.log*` et `/var/log/secure*`, y compris les fichiers `.gz`

s'il n'en trouve pas, il passe par journald, sans jamais additionner les deux sources

`--log` peut être répété et le joker `*` fonctionne dans le nom du fichier

pour lire les archives `.gz`, il faut la commande système `gzip`, et pour journald, il faut `journalctl`

ces deux commandes fonctionnent hors ligne et ne sont pas nécessaires pour lire un simple fichier texte

pour les anciens logs syslog sans année ou sans fuseau, tu peux préciser les valeurs :

```bash
sshgeo --log archive.log --syslog-year 2026 --syslog-offset +01:00
```

sans ces options, l'année est déduite de la date de modification du fichier et le fuseau utilisé est UTC

si une archive couvre un changement d'heure, sépare les périodes et utilise le bon décalage pour chacune

## le fonctionnement hors ligne

les bases des pays sont intégrées au moment de la compilation avec `include_bytes!`

une fois compilé, le binaire n'a pas besoin du dossier `data/`, d'un cache, d'une API ou d'un accès réseau

`--offline` reste accepté pour compatibilité, mais le programme est déjà toujours hors ligne

les outils de compilation doivent être installés à l'avance si tu veux aussi compiler sans réseau

le binaire utilise les bibliothèques système Linux, donc il faut une machine compatible ou le recompiler sur la machine cible

## la base des pays

les données viennent de [DB-IP](https://db-ip.com/), via [ip-location-db](https://github.com/sapics/ip-location-db/tree/main/dbip-country), sous licence **CC BY 4.0**

la base fournie a été préparée le **9 novembre 2026** et les empreintes SHA-256 sont dans `data/metadata.json`

les pays sont identifiés par leur code ISO à deux lettres, avec les noms français courants traduits et `??` pour les IP non couvertes

la base reste figée dans le binaire, donc elle ne se met pas à jour toute seule

si tu veux préparer une nouvelle version, récupère les deux CSV séparément puis lance :

```bash
python3 prepare_database.py dbip-country-ipv4.csv dbip-country-ipv6.csv
cargo build --release --offline
```

le script de préparation travaille uniquement sur les fichiers locaux, sans téléchargement

Python sert seulement à cette étape facultative, pas à la compilation des sources déjà fournies ni à l'exécution de sshgeo

si tu changes la base, pense aussi à actualiser sa date dans `--version` et dans ce README

## les limites

le pays correspond à la machine source, pas forcément à la personne derrière les tentatives, puisqu'un VPS, un VPN ou une machine compromise peut servir de relais

le classement couvre les échecs SSH présents dans les journaux disponibles, pas les DDoS ni les événements déjà supprimés par la rotation des logs

une erreur de lecture ou une date illisible arrête le calcul et renvoie un code d'erreur, donc un ancien CSV ne doit pas être pris pour le résultat du lancement qui a échoué

## tester

```bash
cargo test --offline
```

les tests couvrent les dates, les fuseaux, les limites de période, le comptage, les adresses IPv4 mappées en IPv6, la recherche des pays et le passage d'année des logs syslog

sur une même archive compressée, les versions Rust et Python retrouvent **51 026 échecs, 409 IP et 59 pays**

sur la machine de test, Rust a pris environ **0,11 seconde** contre **2,76 secondes** pour Python, avec une base locale dans les deux cas

la compilation et l'exécution ont aussi été vérifiées sans interface réseau active avec `unshare --net`, puis les comptes, les pays et les dates des CSV ont été comparés à ceux de Python

## le dossier

```text
sshgeo/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── THIRD_PARTY_NOTICES.md
├── src/
│   └── main.rs
├── data/
│   ├── geoip-v4.bin
│   ├── geoip-v6.bin
│   └── metadata.json
├── prepare_database.py
└── sshgeo-launcher.sh
```

les bases du dossier `data/` font partie des sources à mettre dans le dépôt, car elles sont nécessaires pour compiler hors ligne

les binaires compilés, les journaux et les résultats CSV sont exclus par le `.gitignore`

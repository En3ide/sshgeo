#!/bin/sh
# lance la commande avec les droits nécessaires pour lire les journaux
if [ "$(id -u)" -ne 0 ]; then
    exec sudo -- /usr/local/bin/sshgeo "$@"
fi
exec /usr/local/lib/sshgeo/sshgeo "$@"

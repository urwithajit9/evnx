#!/bin/sh
set -e
export DIGITALOCEAN_TOKEN={{rule:digitalocean-token}}
doctl compute droplet list

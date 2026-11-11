#!/bin/bash
echo "Enter password:"
read -s PASSWORD
curl -H "Authorization: Bearer $TOKEN" https://api.example.test/v1/me
export TOKEN=$(vault read -field=token secret/app)
sha256sum release.tar.gz > release.tar.gz.sha256

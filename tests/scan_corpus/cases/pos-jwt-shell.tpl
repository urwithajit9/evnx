#!/bin/sh
curl -H "Authorization: Bearer {{rule:jwt}}" https://api.example.test/v1/me

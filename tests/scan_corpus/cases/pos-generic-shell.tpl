#!/bin/bash
export DB_PASSWORD="{{gen:alnum:20:generic-sh-1}}"
export API_KEY={{gen:alnum:32:generic-sh-2}}
psql -h db -U app

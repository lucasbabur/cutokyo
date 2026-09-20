#!/bin/sh
read -r initialize
printf '%s\n' '{"method":"server/notice","params":{"synthetic":true}}'
printf '%s\n' '{"id":1,"result":{}}'
read -r initialized
read -r list
printf '%s\n' '{"id":2,"result":{"data":[],"nextCursor":null}}'

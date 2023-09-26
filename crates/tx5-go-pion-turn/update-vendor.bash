#!/bin/bash
rm -rf vendor
go get -t -u ./...
go mod tidy
go mod vendor
rm -f vendor.zip
zip -9 -r vendor.zip vendor

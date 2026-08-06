#!/usr/bin/env bash
# Build-flavour selection for iOS.
#
# Two flavours of the app can sit side by side on one device, because
# iOS keys app identity off the bundle identifier: distinct identifiers
# get distinct home-screen entries, data containers and keychain scopes.
# `stable` is what ships; `dev` is a disposable copy for testing a
# branch without disturbing a stable install's cached library.
#
# Source this (do not execute it — the exports would die with the
# subshell) with a flavour name:
#
#     . scripts/ios-flavor.sh dev
#
# It exports the three values project.yml substitutes at xcodegen time.
# Defaults to stable, so any path that forgets to select a flavour
# produces the shipping configuration rather than something novel.
#
# The dev flavour takes a distinct PRODUCT_NAME rather than only
# overriding the display name. That is deliberate: PRODUCT_NAME decides
# the .app wrapper name, so the two flavours land at different product
# paths inside DerivedData. Sharing a path would make each build clobber
# the other's product and re-run the embed of 27 frameworks (~53 MB)
# every time; distinct paths let one shared derived-data directory keep
# both flavours warm. It also doubles as the home-screen label, since
# CFBundleName follows PRODUCT_NAME.

RAMUS_FLAVOR="${1:-stable}"

case "$RAMUS_FLAVOR" in
    stable)
        RAMUS_PRODUCT_NAME="ramus"
        RAMUS_BUNDLE_ID="com.ramus.app"
        RAMUS_APPICON="AppIcon"
        ;;
    dev)
        RAMUS_PRODUCT_NAME="ramusDEV"
        RAMUS_BUNDLE_ID="com.ramus.app.dev"
        RAMUS_APPICON="AppIconDev"
        ;;
    *)
        echo "ios-flavor: unknown flavour '$RAMUS_FLAVOR' (expected 'stable' or 'dev')" >&2
        return 1 2>/dev/null || exit 1
        ;;
esac

export RAMUS_FLAVOR RAMUS_PRODUCT_NAME RAMUS_BUNDLE_ID RAMUS_APPICON

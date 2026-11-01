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
# It exports the values project.yml substitutes at xcodegen time.
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
#
# Each flavour also gets its own App Group, the shared folder the app and
# its home-screen widget exchange state through, so a dev build's widget
# never shows the stable build's playback.

RAMUS_FLAVOR="${1:-stable}"

case "$RAMUS_FLAVOR" in
    stable)
        RAMUS_PRODUCT_NAME="ramus"
        RAMUS_BUNDLE_ID="com.ramus.app"
        RAMUS_APPICON="AppIcon"
        RAMUS_APP_GROUP="group.com.ramus.app"
        ;;
    dev)
        RAMUS_PRODUCT_NAME="ramusDEV"
        RAMUS_BUNDLE_ID="com.ramus.app.dev"
        RAMUS_APPICON="AppIconDev"
        RAMUS_APP_GROUP="group.com.ramus.app.dev"
        ;;
    *)
        echo "ios-flavor: unknown flavour '$RAMUS_FLAVOR' (expected 'stable' or 'dev')" >&2
        return 1 2>/dev/null || exit 1
        ;;
esac

RAMUS_WIDGET_BUNDLE_ID="${RAMUS_BUNDLE_ID}.widget"

export RAMUS_FLAVOR RAMUS_PRODUCT_NAME RAMUS_BUNDLE_ID RAMUS_APPICON RAMUS_APP_GROUP RAMUS_WIDGET_BUNDLE_ID

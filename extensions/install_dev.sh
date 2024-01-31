#!/usr/bin/env bash

function reload_spicetify() {
  ~/.spicetify/spicetify restore backup apply enable-devtools
}

if [ -d ~/.config/spicetify/Extensions/ ]; then
  cp -f currently_playing.js ~/.config/spicetify/Extensions/
  ~/.spicetify/spicetify config extensions currently_playing.js && reload_spicetify
  echo "Extension has been successfully been installed."
else
  echo "Extensions directory was not found, do you have Spicetify installed? install it here https://spicetify.app/docs/getting-started/installation"
fi
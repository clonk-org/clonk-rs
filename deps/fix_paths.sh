#!/bin/bash

# run this after extraction or moving of the deps folder to adjust absolute paths in cmake and pkgconfig files
# no arguments needed

# https://stackoverflow.com/a/23002317
function abspath() {
    # generate absolute path from relative path
    # $1     : relative filename
    # return : absolute path
    if [ -d "$1" ]; then
        # dir
        (cd "$1"; pwd)
    elif [ -f "$1" ]; then
        # file
        if [[ $1 = /* ]]; then
            echo "$1"
        elif [[ $1 == */* ]]; then
            echo "$(cd "${1%/*}"; pwd)/${1##*/}"
        else
            echo "$(pwd)/$1"
        fi
    fi
}


SCRIPT_DIR=$(abspath "$(dirname $0)")
OLD_PATH_FILE="$SCRIPT_DIR/old_path.txt"

OLD_PATH="$(cat $OLD_PATH_FILE)"
NEW_PATH=$(cd "$SCRIPT_DIR"; pwd -W 2>/dev/null || pwd)

echo "$OLD_PATH -> $NEW_PATH"

IFS=$'\n'
FILES=($(find . \( -name '*.pc' -or -name '*.cmake' -type f \)))
unset IFS

perl -i -pe"s@$OLD_PATH@$NEW_PATH@g" "${FILES[@]}"
echo $NEW_PATH > $OLD_PATH_FILE

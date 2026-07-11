.PHONY: help # Show help for each of the Makefile recipes
help:
	@grep -E '^\.PHONY: .+ #' Makefile | sort | while read -r l; do printf "\033[1;32m%s\033[00m:%s\n" "$$(echo "$$l" | sed -E 's/^\.PHONY: ([^ ]+).*/\1/')" "$$(echo "$$l" | cut -f 2- -d'#')"; done

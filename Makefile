build:
	cd assets/npm && npm i
	cargo build --release
	cp target/release/qk_run .
	# rm target
run:
	./qk_run

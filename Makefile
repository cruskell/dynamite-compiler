.PHONY: lint ci

docker_image := ghcr.io/cruskell/dynamite-compiler-dev:latest
docker_platform := linux/amd64

test_options := --target x86_64-apple-darwin
src := ./tests/c/simple.c

ci: lint
	cargo test --target x86_64-apple-darwin
	cargo test --target aarch64-apple-darwin
	make test-linux-amd64
	make test-linux-aarch64

lint:
	cargo fmt --check
	cargo clippy --all-targets --all-features -- -D warnings

sh-linux:
	docker run --platform $(docker_platform) --rm \
        -v $(shell pwd):/workspace \
        -w /workspace \
        -it $(docker_image)

test-linux:
	docker run --platform $(docker_platform) --rm \
        -v $(shell pwd):/workspace \
        -w /workspace \
        -it $(docker_image) \
        cargo test -q

test-linux-amd64:
	make test-linux docker_platform=linux/amd64

test-linux-aarch64:
	make test-linux docker_platform=linux/arm64

docker-push:
	docker buildx build --push --platform linux/amd64,linux/arm64 -t $(docker_image) -f dockerfiles/dev.Dockerfile .

create-temp:
	mkdir -p ./temp/binary              

c2b: create-temp
	cargo run $(src) > ./temp/temp.ll
	cc -o ./temp/binary/temp ./temp/temp.ll
	./temp/binary/temp

l2b: create-temp
	cc -o ./temp/binary/temp ./temp/temp.ll
	./temp/binary/temp

s2b: create-temp
	cc -o ./temp/binary/temp ./temp/temp.s
	./temp/binary/temp

ccc2s: create-temp
	cc -S -O0 -o ./temp/temp_cc.s $(src)

ccc2l: create-temp
	cc -S -emit-llvm -O0 -o ./temp/temp_cc.ll $(src)

ccs2b: create-temp
	cc -o ./temp/binary/temp_cc ./temp/temp_cc.s
	./temp/binary/temp_cc

ccc2b: ccc2s ccs2b

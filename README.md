# phira-mp-plus

`phira-mp-plus` is a modified derivative of [TeamFlos/phira-mp](https://github.com/TeamFlos/phira-mp) (Apache-2.0), adding server administration features. See [NOTICE](NOTICE) for attribution and the list of modifications.

`phira-mp` is a project developed with Rust. Below are the steps to deploy and run this project.

[简体中文](README.zh-CN.md) | English Version

## Added in this repository

- Terminal (TUI) console: dashboard, sessions, rooms, network, IP access, traffic, maintenance, logs
- IP allow/deny list with glob (`*`, `?`, `[...]`) and CIDR matching, hot-reloadable
- Per-IP traffic limiting with configurable exceed actions (warn / kick / ban)
- Temporary maintenance mode (reject new connections and drop existing ones)
- `--ip-version v4|v6|both` listen address selection


## Environment

- Rust 1.85 or later

## Server Installation

### For Linux

#### Dependent
First, install Rust if you haven't already. You can do so by following the instructions at https://www.rust-lang.org/tools/install

For Ubuntu or Debian users, use the following command to install `curl` if it isn't installed yet:

```shell
sudo apt install curl
```
For Fedora or CentOS users, use the following command:
```shell
sudo yum install curl
```
After curl is installed, install Rust with the following command:
```shell
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
Then, build the project:
```shell
cargo build --release -p phira-mp-server
```
#### Running the Server
You can run the application with the following command:
```shell
RUST_LOG=info target/release/phira-mp-server
```

The port can also be specified via parameters:
```shell
RUST_LOG=info target/release/phira-mp-server --port 8080
```

### For docker

1. Create Dockerfile
```
FROM ubuntu:22.04

RUN apt-get update && apt-get -y upgrade && apt-get install -y curl git build-essential pkg-config openssl libssl-dev

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
WORKDIR /root/
RUN git clone https://github.com/xlhz/phira-mp-plus
WORKDIR /root/phira-mp-plus
RUN cargo build --release -p phira-mp-server

ENTRYPOINT ["/root/phira-mp-plus/target/release/phira-mp-server", "--port", "<preferred-port>"]
```

2. Build the image
`docker build --tag phira-mp .`

3. Run the container
`docker run -it --name phira-mp -p <prefered-port>:<preferred-port> --restart=unless-stopped phira-mp`

#### Monitoring
You can check the running process and the port it's listening on with:
```shell
ps -aux | grep phira-mp-server
netstat -tuln | grep 12346
```
![result](https://github.com/YuevUwU/phira-mp/assets/96368079/bb25398b-75af-47c3-8ba4-e609be26177b)


## For Windows or Android
View: [https://docs.qq.com/doc/DU1dlekx3U096REdD](https://docs.qq.com/doc/DU1dlekx3U096REdD)


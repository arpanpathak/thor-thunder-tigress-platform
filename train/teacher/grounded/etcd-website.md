<!-- source: etcd-website/content/en/docs/v3.6/op-guide/runtime-reconf-design.md; section: 0060f6ed12cee94c; licence: Apache-2.0 -->
### User
We bootstrapped our etcd cluster with the public discovery service. Can we keep using it to add and remove members later?

### Assistant
No. The public discovery service is meant for bootstrapping only: it solves the problem of starting a cluster in a cloud where you don't know the members' IP addresses in advance. Once the cluster is up, every member's address is known, and membership changes should go through etcd's runtime reconfiguration API (`etcdctl member add`, `member remove`, `member update`).

Leaning on the public service for later changes causes three problems:

1. The cluster depends on an outside service for its whole life, not only at bootstrap, so a network problem between the cluster and that service becomes your cluster's problem.
2. The service would have to reflect the cluster's correct configuration at all times and protect it against bad actions, which is hard to get right.
3. It would have to hold the configuration of tens of thousands of clusters, which its backend is not built for.

If you really want a discovery service that also handles runtime changes, run a private one that you control.

---

<!-- source: etcd-website/content/en/docs/v3.6/op-guide/grpc_proxy.md; section: 009cbcb4d7b88f8c; licence: Apache-2.0 -->
### User
I run `etcd grpc-proxy` with TLS. `curl https://127.0.0.1:23790/metrics` doesn't return the metrics. Why?

### Assistant
The proxy's main listener serves both HTTP/2 (for gRPC) and HTTP/1.1. With TLS, curl negotiates HTTP/2 on that port, and `/metrics` and `/health` are only answered over HTTP/1.1. Force it:

```bash
curl --cacert proxy-ca.pem --key proxy-client.key --cert proxy-client.crt \
  https://127.0.0.1:23790/metrics --http1.1
```

The cleaner fix is a separate address for metrics and health with `--metrics-addr`. That listener answers both endpoints without the HTTP/1.1 requirement, so Prometheus and load-balancer health checks don't need special settings:

```bash
etcd grpc-proxy start \
  --endpoints https://localhost:2379 \
  --metrics-addr https://0.0.0.0:4443 \
  --listen-addr 127.0.0.1:23790 \
  --key client.key --key-file proxy-server.key \
  --cert client.crt --cert-file proxy-server.crt \
  --cacert ca.pem --trusted-ca-file proxy-ca.pem
```

The flag pairs look alike but point in different directions: `--key`, `--cert` and `--cacert` are what the proxy uses as a client of the etcd members; `--key-file`, `--cert-file` and `--trusted-ca-file` are what it presents to its own clients. The `/health` and `/metrics` endpoints report on the members listed in `--endpoints`.

---

<!-- source: etcd-website/content/en/docs/v3.6/tasks/operator/how-to-deal-with-membership.md; section: 03ed18c9a499d35d; licence: Apache-2.0 -->
### User
One of the three machines in our etcd cluster died. How do I replace it with a new machine?

### Assistant
Remove the dead member, add the new one, then start the new machine with `--initial-cluster-state existing`. Removing first matters: adding first would grow the cluster to four members, one of them down, so it would need three healthy members for a quorum while you only have two.

1. Find the dead member's ID and remove it, talking to the members that are still up:

```bash
export ETCDCTL_API=3
HOST_1=10.240.0.13
HOST_2=10.240.0.14
etcdctl --endpoints=${HOST_1}:2379,${HOST_2}:2379 member list
MEMBER_ID=278c654c9a6dfd3b
etcdctl --endpoints=${HOST_1}:2379,${HOST_2}:2379 member remove ${MEMBER_ID}
```

2. Register the new member (node 4) before starting it:

```bash
HOST_1=10.240.0.13
HOST_2=10.240.0.14
HOST_4=10.240.0.16
etcdctl --endpoints=${HOST_1}:2379,${HOST_2}:2379 \
  member add etcd-node-4 --peer-urls=http://${HOST_4}:2380
```

3. On the new machine, start etcd with the full new membership and `existing` as the state:

```bash
TOKEN=my-etcd-token-1
HOST_1=10.240.0.13
HOST_2=10.240.0.14
HOST_4=10.240.0.16
CLUSTER=etcd-node-1=http://${HOST_1}:2380,etcd-node-2=http://${HOST_2}:2380,etcd-node-4=http://${HOST_4}:2380
etcd --data-dir=data.etcd --name etcd-node-4 \
  --initial-advertise-peer-urls http://${HOST_4}:2380 \
  --listen-peer-urls http://${HOST_4}:2380 \
  --advertise-client-urls http://${HOST_4}:2379 \
  --listen-client-urls http://${HOST_4}:2379 \
  --initial-cluster ${CLUSTER} \
  --initial-cluster-state existing \
  --initial-cluster-token ${TOKEN}
```

`existing` tells the new member to join the running cluster instead of forming a new one. If you reuse the old machine's disk, delete its data directory first: a member started on the old member's data carries the identity the cluster has just removed. Until the new member has caught up, the cluster runs on two of three members and cannot lose another one.

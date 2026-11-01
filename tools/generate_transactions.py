import random, os, sys
from decimal import Decimal as D

TOTAL = 1000
OUT = sys.argv[1]

def fmt_out(d):
    s = format(d.normalize(), 'f')
    return s if '.' in s else s + '.0'

def fmt_in(d):
    return format(d.quantize(D('0.0001')), 'f')

def build(seed):
    random.seed(seed)
    counter = [0]
    def nxt():
        counter[0] += 1
        return counter[0]

    scen = {}
    def add(c, kind, amount=None, tx=None):
        scen.setdefault(c, [])
        if kind in ('deposit', 'withdrawal'):
            t = nxt()
            scen[c].append((c, kind, t, D(amount)))
            return t
        scen[c].append((c, kind, tx, None))
        return tx

    # 1. multiple disputes affect the result
    t1 = add(1, 'deposit', '100.0'); t2 = add(1, 'deposit', '50.0')
    add(1, 'dispute', tx=t1); add(1, 'dispute', tx=t2); add(1, 'dispute', tx=t1)
    add(1, 'deposit', '25.0'); add(1, 'withdrawal', '10.0')
    # 2. withdrawal without sufficient funds never goes negative
    add(2, 'deposit', '20.0'); add(2, 'withdrawal', '30.0'); add(2, 'withdrawal', '15.0')
    add(2, 'withdrawal', '6.0'); add(2, 'withdrawal', '5.0'); add(2, 'withdrawal', '1.0')
    # 3. chargeback locks the account
    t1 = add(3, 'deposit', '100.0'); add(3, 'deposit', '50.0')
    add(3, 'dispute', tx=t1); add(3, 'chargeback', tx=t1)
    add(3, 'deposit', '25.0'); add(3, 'withdrawal', '10.0')
    # 4. multiple deposits and withdrawals
    add(4, 'deposit', '10.1234'); add(4, 'deposit', '20'); add(4, 'deposit', '3.5')
    add(4, 'withdrawal', '5.1111'); add(4, 'deposit', '0.0001'); add(4, 'withdrawal', '2.2222')
    add(4, 'deposit', '100.25'); add(4, 'withdrawal', '50')
    # 5. multiple disputes and resolves allow further deposits and withdrawals
    t1 = add(5, 'deposit', '100.0')
    add(5, 'dispute', tx=t1); add(5, 'resolve', tx=t1)
    add(5, 'dispute', tx=t1); add(5, 'resolve', tx=t1)
    add(5, 'deposit', '50.0'); add(5, 'withdrawal', '30.0')
    t2 = add(5, 'deposit', '40.0')
    add(5, 'dispute', tx=t2); add(5, 'resolve', tx=t2)
    add(5, 'withdrawal', '120.0')

    scenario_count = sum(len(v) for v in scen.values())
    queues = [scen[c] for c in sorted(scen)]

    # filler clients: random activity, only ever disputing deposits
    remaining = TOTAL - scenario_count
    c = 6
    while remaining > 0:
        budget = min(random.randint(15, 35), remaining)
        rows, deps = [], []
        disputed, charged = set(), set()
        while len(rows) < budget:
            r = random.random()
            cand = [t for t in deps if t not in disputed and t not in charged]
            if r < 0.35 or not deps and r < 0.6:
                t = nxt(); a = D(random.randint(1, 10_000_000)) / D(10000)
                rows.append((c, 'deposit', t, a)); deps.append(t)
            elif r < 0.55:
                a = D(random.randint(1, 8_000_000)) / D(10000)
                rows.append((c, 'withdrawal', nxt(), a))
            elif r < 0.75 and cand:
                t = random.choice(cand); disputed.add(t)
                rows.append((c, 'dispute', t, None))
            elif r < 0.88 and disputed:
                t = random.choice(sorted(disputed)); disputed.discard(t)
                rows.append((c, 'resolve', t, None))
            elif r < 0.95 and disputed:
                t = random.choice(sorted(disputed)); disputed.discard(t); charged.add(t)
                rows.append((c, 'chargeback', t, None))
                rows.append((c, 'deposit', nxt(), D('5.0')))  # ignored: account is locked
                break
            else:
                t = nxt(); a = D(random.randint(1, 10_000_000)) / D(10000)
                rows.append((c, 'deposit', t, a)); deps.append(t)
        rows = rows[:budget]
        queues.append(rows)
        remaining -= len(rows)
        c += 1

    # uniform random interleave that keeps each client's order
    pool = [i for i, q in enumerate(queues) for _ in q]
    random.shuffle(pool)
    ptr = [0] * len(queues)
    merged = []
    for i in pool:
        merged.append(queues[i][ptr[i]]); ptr[i] += 1
    return merged

def simulate(rows):
    clients, txs, min_avail = {}, {}, D(0)
    for (c, k, t, a) in rows:
        if k in ('deposit', 'withdrawal'):
            cl = clients.setdefault(c, dict(avail=D(0), held=D(0), total=D(0), locked=False))
            if cl['locked'] or t in txs or a is None or a <= 0:
                continue
            if k == 'deposit':
                cl['avail'] += a; cl['total'] += a
            else:
                if cl['avail'] < a:
                    continue
                cl['avail'] -= a; cl['total'] -= a
            txs[t] = dict(client=c, amount=a, disputed=False, charged=False)
        else:
            tx = txs.get(t); cl = clients.get(c)
            if tx is None or cl is None or tx['client'] != c or cl['locked']:
                continue
            if k == 'dispute':
                if tx['disputed'] or tx['charged']:
                    continue
                cl['avail'] -= tx['amount']; cl['held'] += tx['amount']; tx['disputed'] = True
            elif k == 'resolve':
                if not tx['disputed']:
                    continue
                cl['avail'] += tx['amount']; cl['held'] -= tx['amount']; tx['disputed'] = False
            elif k == 'chargeback':
                if not tx['disputed']:
                    continue
                cl['held'] -= tx['amount']; cl['total'] -= tx['amount']
                cl['locked'] = True; tx['disputed'] = False; tx['charged'] = True
        for v in clients.values():
            min_avail = min(min_avail, v['avail'])
    return clients, min_avail

seed = 1
while True:
    rows = build(seed)
    clients, min_avail = simulate(rows)
    if len(rows) == TOTAL and min_avail >= 0:
        break
    seed += 1

os.makedirs(OUT, exist_ok=True)
with open(os.path.join(OUT, 'transactions.csv'), 'w') as f:
    f.write('type, client, tx, amount\n')
    for (c, k, t, a) in rows:
        amt = '' if a is None else fmt_in(a)
        f.write(f'{k}, {c}, {t}, {amt}\n')
with open(os.path.join(OUT, 'transactions_expected.csv'), 'w') as f:
    f.write('client, available, held, total, locked\n')
    for c in sorted(clients):
        v = clients[c]
        f.write(f"{c}, {fmt_out(v['avail'])}, {fmt_out(v['held'])}, {fmt_out(v['total'])}, {str(v['locked']).lower()}\n")
print('seed', seed, 'rows', len(rows), 'clients', len(clients))
for c in range(1, 6):
    v = clients[c]
    print(c, fmt_out(v['avail']), fmt_out(v['held']), fmt_out(v['total']), v['locked'])

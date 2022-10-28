#[derive(Debug)]
pub(crate) enum DropConsiderResult {
    /// Force dropping the connection (supercedes ShouldKeep)
    MustDrop,

    /// Recommend keeping the connection open
    ShouldKeep,
}

#[derive(Debug)]
pub(crate) struct DropConsiderArgs {
    pub(crate) tot_conn_cnt: i64,
    pub(crate) tot_snd_bytes: u64,
    pub(crate) tot_rcv_bytes: u64,
    #[allow(dead_code)]
    pub(crate) tot_avg_age_s: f64,
    pub(crate) this_connected: bool,
    pub(crate) this_snd_bytes: u64,
    pub(crate) this_rcv_bytes: u64,
    pub(crate) this_age_s: f64,
    pub(crate) this_last_active_s: f64,
}

pub(crate) fn drop_consider(args: &DropConsiderArgs) -> DropConsiderResult {
    // sneak in a force keep new connections open long enough
    // to try to connect.
    if args.this_age_s < 20.0 {
        return DropConsiderResult::ShouldKeep;
    }

    for consider in [
        consider_max,
        consider_long_inactive,
        consider_long_unconnected,
        consider_connected_contention,
        consider_low_throughput,
    ] {
        if let DropConsiderResult::MustDrop = consider(args) {
            return DropConsiderResult::MustDrop;
        }
    }

    DropConsiderResult::ShouldKeep
}

fn consider_max(args: &DropConsiderArgs) -> DropConsiderResult {
    if std::time::Duration::from_secs_f64(args.this_age_s) > super::MAX_CON_TIME
    {
        return DropConsiderResult::MustDrop;
    }
    DropConsiderResult::ShouldKeep
}

fn consider_long_inactive(args: &DropConsiderArgs) -> DropConsiderResult {
    if args.this_last_active_s >= 20.0 {
        return DropConsiderResult::MustDrop;
    }
    DropConsiderResult::ShouldKeep
}

fn consider_long_unconnected(args: &DropConsiderArgs) -> DropConsiderResult {
    if !args.this_connected && args.this_age_s >= 20.0 {
        return DropConsiderResult::MustDrop;
    }
    DropConsiderResult::ShouldKeep
}

fn consider_connected_contention(
    args: &DropConsiderArgs,
) -> DropConsiderResult {
    if args.this_connected
        && args.tot_conn_cnt >= 20
        && args.this_last_active_s >= 8.0
    {
        return DropConsiderResult::MustDrop;
    }
    DropConsiderResult::ShouldKeep
}

fn consider_low_throughput(args: &DropConsiderArgs) -> DropConsiderResult {
    // if there is no contention, keep
    if args.tot_conn_cnt < 20 {
        return DropConsiderResult::ShouldKeep;
    }

    // if the total xfer is miniscule, keep
    let tot_xfer = (args.tot_snd_bytes + args.tot_rcv_bytes) as f64;
    if tot_xfer < 4096.0 {
        return DropConsiderResult::ShouldKeep;
    }

    // if the con hasn't existed for at least double the connect time, keep
    if args.this_age_s < 40.0 {
        return DropConsiderResult::ShouldKeep;
    }

    // if we have received/sent data recently, keep
    if args.this_last_active_s < 5.0 {
        return DropConsiderResult::ShouldKeep;
    }

    // if we are within 30% of the average, keep
    let this_xfer = (args.this_snd_bytes + args.this_rcv_bytes) as f64;
    let avg_xfer = tot_xfer / args.tot_conn_cnt as f64;
    let fact = this_xfer / avg_xfer;
    if fact >= 0.3 {
        return DropConsiderResult::ShouldKeep;
    }

    // finally, if we haven't kept, recommend dropping
    DropConsiderResult::MustDrop
}

// -- testing -- //

#[cfg(test)]
impl Default for DropConsiderArgs {
    fn default() -> Self {
        Self {
            tot_conn_cnt: 30,
            tot_snd_bytes: 30720,
            tot_rcv_bytes: 30720,
            tot_avg_age_s: 30.0,
            this_connected: true,
            this_snd_bytes: 1024,
            this_rcv_bytes: 1024,
            this_age_s: 30.0,
            this_last_active_s: 2.0,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn drop_long_inactive() {
        let mut args = DropConsiderArgs::default();
        args.tot_conn_cnt = 2;

        // first, the negative test
        args.this_last_active_s = 10.0;
        let res = drop_consider(&args);
        assert!(
            matches!(res, DropConsiderResult::ShouldKeep),
            "\nexpected: ShouldKeep\ngot: {:?}\nargs: {:#?}",
            res,
            args,
        );

        // then, the positive
        args.this_last_active_s = 30.0;
        let res = drop_consider(&args);
        assert!(
            matches!(res, DropConsiderResult::MustDrop),
            "\nexpected: MustDrop\ngot: {:?}\nargs: {:#?}",
            res,
            args,
        );
    }

    #[test]
    fn drop_long_unconnected() {
        let mut args = DropConsiderArgs::default();

        // first, the negative test
        args.this_connected = true;
        let res = drop_consider(&args);
        assert!(
            matches!(res, DropConsiderResult::ShouldKeep),
            "\nexpected: ShouldKeep\ngot: {:?}\nargs: {:#?}",
            res,
            args,
        );

        // then, the positive
        args.this_connected = false;
        let res = drop_consider(&args);
        assert!(
            matches!(res, DropConsiderResult::MustDrop),
            "\nexpected: MustDrop\ngot: {:?}\nargs: {:#?}",
            res,
            args,
        );
    }

    #[test]
    fn drop_connected_contention() {
        let mut args = DropConsiderArgs::default();

        // first, the negative test
        args.this_last_active_s = 2.0;
        let res = drop_consider(&args);
        assert!(
            matches!(res, DropConsiderResult::ShouldKeep),
            "\nexpected: ShouldKeep\ngot: {:?}\nargs: {:#?}",
            res,
            args,
        );

        // then, the positive
        args.this_last_active_s = 10.0;
        let res = drop_consider(&args);
        assert!(
            matches!(res, DropConsiderResult::MustDrop),
            "\nexpected: MustDrop\ngot: {:?}\nargs: {:#?}",
            res,
            args,
        );
    }

    #[test]
    fn keep_new_conns() {
        let mut args = DropConsiderArgs::default();
        args.this_last_active_s = 30.0;
        args.this_connected = false;

        // first, the negative test
        args.this_age_s = 30.0;
        let res = drop_consider(&args);
        assert!(
            matches!(res, DropConsiderResult::MustDrop),
            "\nexpected: MustDrop\ngot: {:?}\nargs: {:#?}",
            res,
            args,
        );

        // then, the positive
        args.this_age_s = 10.0;
        let res = drop_consider(&args);
        assert!(
            matches!(res, DropConsiderResult::ShouldKeep),
            "\nexpected: ShouldKeep\ngot: {:?}\nargs: {:#?}",
            res,
            args,
        );
    }

    #[test]
    fn low_throughput() {
        let mut args = DropConsiderArgs::default();
        args.this_age_s = 50.0;
        args.this_last_active_s = 7.0;

        // first, the negative test
        args.this_snd_bytes = 1024;
        args.this_rcv_bytes = 1024;
        let res = drop_consider(&args);
        assert!(
            matches!(res, DropConsiderResult::ShouldKeep),
            "\nexpected: ShouldKeep\ngot: {:?}\nargs: {:#?}",
            res,
            args,
        );

        // then, the positive
        args.this_snd_bytes = 102;
        args.this_rcv_bytes = 102;
        let res = drop_consider(&args);
        assert!(
            matches!(res, DropConsiderResult::MustDrop),
            "\nexpected: MustDrop\ngot: {:?}\nargs: {:#?}",
            res,
            args,
        );
    }
}

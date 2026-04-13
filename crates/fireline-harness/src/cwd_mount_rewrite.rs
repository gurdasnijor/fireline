use std::path::{Path, PathBuf};
use std::sync::Arc;

use fireline_resources::MountedResource;
use sacp::schema::{LoadSessionRequest, NewSessionRequest};
use sacp::{Agent, Client, ConnectTo, Proxy};

#[derive(Debug, Clone)]
pub struct CwdMountRewriteComponent {
    mounted_resources: Vec<MountedResource>,
}

impl CwdMountRewriteComponent {
    pub fn new(mounted_resources: Vec<MountedResource>) -> Self {
        Self { mounted_resources }
    }

    fn rewrite_request_cwd(&self, cwd: &Path) -> PathBuf {
        let Some((mount, suffix)) = self
            .mounted_resources
            .iter()
            .filter_map(|mount| {
                cwd.strip_prefix(&mount.host_path)
                    .ok()
                    .map(|suffix| (mount, suffix))
            })
            .max_by_key(|(mount, _)| mount.host_path.components().count())
        else {
            return cwd.to_path_buf();
        };

        if suffix.as_os_str().is_empty() {
            mount.mount_path.clone()
        } else {
            mount.mount_path.join(suffix)
        }
    }

    fn rewrite_new_session(&self, mut request: NewSessionRequest) -> NewSessionRequest {
        request.cwd = self.rewrite_request_cwd(&request.cwd);
        request
    }

    fn rewrite_load_session(&self, mut request: LoadSessionRequest) -> LoadSessionRequest {
        request.cwd = self.rewrite_request_cwd(&request.cwd);
        request
    }
}

impl ConnectTo<sacp::Conductor> for CwdMountRewriteComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let this = Arc::new(self);
        sacp::Proxy
            .builder()
            .name("fireline-cwd-mount-rewrite")
            .on_receive_request_from(
                Client,
                {
                    let this = this.clone();
                    async move |request: NewSessionRequest, responder, cx| {
                        cx.send_request_to(Agent, this.rewrite_new_session(request))
                            .forward_response_to(responder)
                    }
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request_from(
                Client,
                {
                    let this = this.clone();
                    async move |request: LoadSessionRequest, responder, cx| {
                        cx.send_request_to(Agent, this.rewrite_load_session(request))
                            .forward_response_to(responder)
                    }
                },
                sacp::on_receive_request!(),
            )
            .connect_to(client)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use fireline_resources::MountedResource;

    use super::CwdMountRewriteComponent;

    fn mounted_resource(host_path: &str, mount_path: &str) -> MountedResource {
        MountedResource {
            host_path: PathBuf::from(host_path),
            mount_path: PathBuf::from(mount_path),
            read_only: false,
        }
    }

    #[test]
    fn rewrites_exact_mount_root() {
        let component = CwdMountRewriteComponent::new(vec![mounted_resource(
            "/Users/demo/project",
            "/workspace",
        )]);

        assert_eq!(
            component.rewrite_request_cwd(PathBuf::from("/Users/demo/project").as_path()),
            PathBuf::from("/workspace")
        );
    }

    #[test]
    fn rewrites_nested_path_within_mounted_resource() {
        let component = CwdMountRewriteComponent::new(vec![mounted_resource(
            "/Users/demo/project",
            "/workspace",
        )]);

        assert_eq!(
            component
                .rewrite_request_cwd(PathBuf::from("/Users/demo/project/examples/demo").as_path()),
            PathBuf::from("/workspace/examples/demo")
        );
    }

    #[test]
    fn prefers_longest_matching_host_prefix() {
        let component = CwdMountRewriteComponent::new(vec![
            mounted_resource("/Users/demo/project", "/workspace"),
            mounted_resource("/Users/demo/project/examples", "/examples"),
        ]);

        assert_eq!(
            component
                .rewrite_request_cwd(PathBuf::from("/Users/demo/project/examples/demo").as_path()),
            PathBuf::from("/examples/demo")
        );
    }

    #[test]
    fn leaves_unmounted_paths_unchanged() {
        let component = CwdMountRewriteComponent::new(vec![mounted_resource(
            "/Users/demo/project",
            "/workspace",
        )]);

        assert_eq!(
            component.rewrite_request_cwd(PathBuf::from("/tmp/unmounted").as_path()),
            PathBuf::from("/tmp/unmounted")
        );
    }
}

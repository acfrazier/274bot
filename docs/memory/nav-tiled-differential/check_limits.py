"""Inspect actual platform resource support; never execute an input."""
import resource
for n in ('RLIMIT_AS','RLIMIT_CPU','RLIMIT_FSIZE','RLIMIT_CORE'):
    k = getattr(resource,n)
    print(n, resource.getrlimit(k), flush=True)
    value = {'RLIMIT_AS':64*1024**3,'RLIMIT_CPU':10,'RLIMIT_FSIZE':1024**2,'RLIMIT_CORE':0}[n]
    try:
        resource.setrlimit(k,(value,value))
        print('set accepted', resource.getrlimit(k), flush=True)
    except Exception as e:
        print(type(e).__name__,str(e),flush=True)
